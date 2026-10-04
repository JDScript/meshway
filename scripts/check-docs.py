#!/usr/bin/env python3
"""Check rendered V2 links/anchors and the published generated contract."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import unquote, urljoin, urlsplit
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "docs/.vitepress/dist"


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        values = dict(attrs)
        if values.get("id"):
            self.ids.add(values["id"])
        for field in (["href"] if tag in ("a", "link") else ["src"] if tag in ("img", "script") else []):
            if values.get(field):
                self.links.append(values[field])


def external_status(url):
    try:
        with urlopen(Request(url, headers={"User-Agent": "Meshway-docs-link-check"}), timeout=15) as response:
            return url, response.status
    except HTTPError as error:
        return url, error.code
    except (URLError, TimeoutError, OSError):
        return url, "unreachable"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--external", action="store_true", help="also GET unique external links")
    args = parser.parse_args()
    paths = sorted((DIST / "v2").rglob("*.html"))
    if not paths:
        raise SystemExit("Build docs before checking links.")
    pages = {path: Page(path) for path in DIST.rglob("*.html")}
    errors, external = [], set()
    checked = 0
    for path in paths:
        relative = path.relative_to(DIST).as_posix()
        route = "/" + (relative[:-10] if relative.endswith("index.html") else relative[:-5])
        for link in pages[path].links:
            parsed = urlsplit(urljoin("https://docs.meshway.invalid" + route, link))
            if parsed.scheme not in ("http", "https"):
                continue
            if parsed.netloc != "docs.meshway.invalid":
                external.add(parsed._replace(fragment="").geturl())
                continue
            destination = DIST / unquote(parsed.path).lstrip("/")
            if not destination.resolve().is_relative_to(DIST.resolve()):
                errors.append(f"{relative}: outside docs: {link}")
                continue
            if not destination.is_file():
                clean_page = Path(str(destination) + ".html")
                destination = clean_page if not parsed.path.endswith("/") and clean_page.is_file() else destination / "index.html"
            checked += 1
            if not destination.is_file():
                errors.append(f"{relative}: missing target: {link}")
            elif parsed.fragment and destination.suffix == ".html" and unquote(parsed.fragment) not in pages[destination].ids:
                errors.append(f"{relative}: missing anchor: {link}")
    artifact = DIST / "v2/contracts/openapi.json"
    if not artifact.is_file() or artifact.read_bytes() != (ROOT / "packages/protocol/openapi.json").read_bytes():
        errors.append("Published OpenAPI differs from the generated contract.")
    for error in sorted(set(errors)):
        print(error)
    print(f"Checked {len(paths)} pages, {checked} local references, {len(external)} external URLs.")
    if args.external:
        with ThreadPoolExecutor(max_workers=4) as workers:
            for url, status in workers.map(external_status, sorted(external)):
                print(f"{status}: {url}")
                if status in (404, 410) or status == "unreachable":
                    errors.append(f"External link failed: {url}")
                elif isinstance(status, int) and status >= 400:
                    errors.append(f"External link could not be verified ({status}): {url}")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
