"""Private journal and in-place, ownership-checked resolv.conf transaction."""

import base64
import fcntl
import hashlib
import ipaddress
import json
import os
import secrets
import stat

LIMIT = 65536


class Conflict(RuntimeError):
    pass


def directory(path, create=False):
    """Walk with openat/O_NOFOLLOW: do not follow parent or final symlinks."""
    path = os.path.abspath(path)
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.split("/")[1:]:
            if not part:
                continue
            if create:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                except FileExistsError:
                    pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def secure_open(path, flags):
    parent, leaf = os.path.split(os.path.abspath(path))
    fd = directory(parent)
    try:
        return os.open(leaf, flags | os.O_NOFOLLOW, dir_fd=fd)
    finally:
        os.close(fd)


def read_fd(fd):
    os.lseek(fd, 0, os.SEEK_SET)
    data = os.read(fd, LIMIT + 1)
    if len(data) > LIMIT:
        raise Conflict("resolver file exceeds 64 KiB")
    return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(data):
    return base64.b64encode(data).decode("ascii")


def decoded(value):
    try:
        data = base64.b64decode(value, validate=True)
    except (ValueError, TypeError) as exc:
        raise Conflict("corrupt journal bytes") from exc
    if len(data) > LIMIT:
        raise Conflict("journal resolver bytes exceed limit")
    return data


def process_start(pid):
    try:
        # comm can include spaces/parentheses, so split after the last ')'.
        with open(f"/proc/{pid}/stat", encoding="ascii") as handle:
            return handle.read().rsplit(")", 1)[1].split()[19]
    except (OSError, IndexError):
        return None


class Store:
    def __init__(self, path, create=True):
        self.fd = directory(path, create=create)
        self.lock_fd = None
        info = os.fstat(self.fd)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
            self.close()
            raise Conflict("state directory must be owned by this user with mode 0700")

    def private_file(self, name, flags, mode=0o600):
        fd = os.open(name, flags | os.O_NOFOLLOW, mode, dir_fd=self.fd)
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
            os.close(fd)
            raise Conflict(f"{name} must be a private 0600 regular file")
        return fd

    def lock(self):
        self.lock_fd = self.private_file("lock", os.O_RDWR | os.O_CREAT)
        try:
            fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise Conflict("resolver owner is busy; another run/down holds the lock") from exc

    def lock_busy(self):
        try:
            fd = self.private_file("lock", os.O_RDWR)
        except FileNotFoundError:
            return False
        try:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return False
            except BlockingIOError:
                return True
        finally:
            os.close(fd)

    def load(self):
        try:
            fd = self.private_file("journal.json", os.O_RDONLY)
        except FileNotFoundError:
            return None
        try:
            # Base64 grows the two resolver copies past LIMIT.
            raw = os.read(fd, 4 * LIMIT + 1)
            if len(raw) > 4 * LIMIT:
                raise Conflict("journal exceeds limit")
            journal = json.loads(raw)
            if journal["version"] != 1:
                raise Conflict("unsupported journal version")
            original, written = decoded(journal["original"]), decoded(journal["written"])
            if digest(written) != journal["written_sha256"] or digest(original) != journal["original_sha256"]:
                raise Conflict("journal digest mismatch")
            return journal
        except (ValueError, KeyError, TypeError) as exc:
            raise Conflict("corrupt journal") from exc
        finally:
            os.close(fd)

    def save(self, journal):
        name = ".journal-" + secrets.token_hex(8)
        fd = self.private_file(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
        try:
            data = json.dumps(journal, sort_keys=True).encode("utf-8") + b"\n"
            write_all(fd, data)
            os.fsync(fd)
            os.rename(name, "journal.json", src_dir_fd=self.fd, dst_dir_fd=self.fd)
            os.fsync(self.fd)
        finally:
            os.close(fd)
            try:
                os.unlink(name, dir_fd=self.fd)
            except FileNotFoundError:
                pass

    def delete(self):
        os.unlink("journal.json", dir_fd=self.fd)
        os.fsync(self.fd)

    def close(self):
        if self.lock_fd is not None:
            os.close(self.lock_fd)
            self.lock_fd = None
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def write_all(fd, data):
    view = memoryview(data)
    while view:
        count = os.write(fd, view)
        if count <= 0:
            raise OSError("short resolver write")
        view = view[count:]


def endpoint(value):
    try:
        if value.startswith("["):
            host, port = value[1:].split("]:")
        else:
            host, port = value.rsplit(":", 1)
        address, port = ipaddress.ip_address(host), int(port)
        if not 1 <= port <= 65535:
            raise ValueError()
        return str(address), port
    except ValueError as exc:
        raise ValueError("endpoint must be numeric IPv4:port or [IPv6]:port") from exc


def original_config(data, listen, nameservice):
    upstreams, search, options = [], [], []
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeError as exc:
        raise Conflict("resolv.conf must be ASCII") from exc
    for line in lines:
        parts = line.split("#", 1)[0].split(";", 1)[0].split()
        if not parts:
            continue
        if parts[0] == "nameserver":
            if len(parts) != 2:
                raise Conflict("invalid original nameserver directive")
            try:
                address = ipaddress.ip_address(parts[1])
            except ValueError as exc:
                raise Conflict("original nameservers must be numeric IP addresses") from exc
            candidate = (str(address), 53)
            mapped = address.ipv4_mapped if isinstance(address, ipaddress.IPv6Address) else None
            if (candidate in (listen, nameservice)
                    or address.is_unspecified or address.is_multicast
                    or (mapped is not None and (mapped.is_loopback or mapped.is_unspecified or mapped.is_multicast))
                    or (address.is_loopback and str(address) != "127.0.0.11")):
                raise Conflict("unsafe original loopback/self upstream; only Docker 127.0.0.11 is allowed")
            if candidate not in upstreams:
                upstreams.append(candidate)
        elif parts[0] in ("search", "domain"):
            # resolv.conf uses the last search/domain directive.
            search = parts[1:] if parts[0] == "search" else parts[1:2]
        elif parts[0] == "options":
            options.extend(parts[1:])
    if not upstreams:
        raise Conflict("no original DNS upstreams; refusing to capture external DNS")
    return upstreams, search, options


def generated_config(zone, options):
    result = "# Owned by Meshway experimental container DNS adapter\n"
    # A libc search retry through an original domain (solo.old.test) is a
    # multi-label external question: the bare-label refusal cannot stop it.
    # Original search/domain directives survive only in the recovery bytes.
    result += "nameserver 127.0.0.1\nsearch " + zone + "\n"
    if options:
        result += "options " + " ".join(options) + "\n"
    return result.encode("ascii")


class ResolverFile:
    def __init__(self, path):
        self.path = os.path.abspath(path)
        self.fd = secure_open(self.path, os.O_RDWR)
        try:
            self.info = os.fstat(self.fd)
            if not stat.S_ISREG(self.info.st_mode) or self.info.st_nlink != 1:
                raise Conflict("resolv.conf must be a regular, non-hardlinked file (symlinks rejected)")
            self.original = read_fd(self.fd)
        except BaseException:
            self.close()
            raise

    def check_inode(self, journal=None):
        fd = secure_open(self.path, os.O_RDONLY)
        try:
            current, opened = os.fstat(fd), os.fstat(self.fd)
            expected = (journal["device"], journal["inode"]) if journal else (self.info.st_dev, self.info.st_ino)
            if (current.st_dev, current.st_ino) != expected or (opened.st_dev, opened.st_ino) != expected:
                raise Conflict("resolv.conf inode changed; journal retained")
        finally:
            os.close(fd)

    def replace(self, expected, content, mode):
        self.check_inode()
        if read_fd(self.fd) != expected:
            raise Conflict("resolv.conf content changed; refusing to overwrite")
        # Docker's resolv.conf is a bind mount; never rename it.
        os.lseek(self.fd, 0, os.SEEK_SET)
        os.ftruncate(self.fd, 0)
        view, written = memoryview(content), 0
        try:
            while view:
                count = os.write(self.fd, view)
                if count <= 0:
                    raise OSError("short resolver write")
                written += count
                view = view[count:]
            os.fchmod(self.fd, mode)
            os.fsync(self.fd)
            self.check_inode()
            if read_fd(self.fd) != content:
                raise Conflict("concurrent resolver edit detected after write")
        except BaseException:
            # Only undo our exact partial bytes, never a third-party edit.
            self.check_inode()
            if read_fd(self.fd) == content[:written]:
                os.lseek(self.fd, 0, os.SEEK_SET)
                os.ftruncate(self.fd, 0)
                write_all(self.fd, expected)
                os.fchmod(self.fd, mode)
                os.fsync(self.fd)
            raise

    def prepare(self, store, zone, listen, nameservice, prefixes):
        previous = store.load()
        zones = []
        if previous:
            self.check_inode(previous)
            if (previous["phase"] != "restored" or self.original != decoded(previous["original"])
                    or previous["path"] != self.path):
                raise Conflict("existing ownership journal requires down/recovery before run")
            zones = previous["zones"]
        upstreams, _search, options = original_config(self.original, listen, nameservice)
        written = generated_config(zone, options)
        info = self.info
        journal = dict(version=1, phase="prepared", path=self.path,
                       original=encoded(self.original), original_sha256=digest(self.original),
                       written=encoded(written), written_sha256=digest(written),
                       mode=stat.S_IMODE(info.st_mode), device=info.st_dev, inode=info.st_ino,
                       zones=list(dict.fromkeys(zones + [zone])), zone=zone,
                       listen=list(listen), nameservice=list(nameservice),
                       prefixes=[str(p) for p in prefixes], upstreams=upstreams,
                       pid=os.getpid(), process_start=process_start(os.getpid()))
        return journal, previous

    def apply(self, store, journal, previous):
        store.save(journal)  # Durable recovery record precedes the first truncate.
        try:
            self.replace(decoded(journal["original"]), decoded(journal["written"]), journal["mode"])
            journal["phase"] = "active"
            store.save(journal)
        except BaseException:
            # replace handles partial-write rollback. A journal save failure after
            # the full write must also restore through the ownership check.
            current = read_fd(self.fd)
            if current == decoded(journal["written"]):
                self.replace(current, decoded(journal["original"]), journal["mode"])
            if read_fd(self.fd) == decoded(journal["original"]):
                if previous:
                    store.save(previous)
                else:
                    store.delete()
            raise

    def restore(self, store, journal, forget=False):
        if journal["path"] != self.path:
            raise Conflict("resolv.conf path differs from owner journal")
        self.check_inode(journal)
        original, written = decoded(journal["original"]), decoded(journal["written"])
        current = read_fd(self.fd)
        if current == written:
            self.replace(written, original, journal["mode"])
        elif current != original:
            raise Conflict("resolv.conf was modified by a third party; journal retained, no overwrite")
        else:
            os.fchmod(self.fd, journal["mode"])
            os.fsync(self.fd)
        if forget:
            store.delete()
        else:
            journal.update(phase="restored", pid=None, process_start=None)
            store.save(journal)

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
