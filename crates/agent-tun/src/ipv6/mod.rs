//! IPv4-only device safety. Missing per-link sysctls require verified inheritance.
use std::{io, path::Path};

pub(super) fn disable_ipv6(tree: &Path, name: &str) -> io::Result<()> {
    let interface = tree.join("conf").join(name).join("disable_ipv6");
    match std::fs::read_to_string(&interface) {
        Ok(value) if value.trim() == "1" => Ok(()),
        Ok(value) if value.trim() == "0" => {
            std::fs::write(&interface, b"1\n")?;
            if std::fs::read_to_string(&interface)?.trim() != "1" {
                return Err(io::Error::other("TUN IPv6 disable did not take effect"));
            }
            Ok(())
        }
        Ok(_) => Err(io::Error::other("invalid TUN IPv6 sysctl value")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !tree.try_exists()? {
                return Ok(());
            }
            match std::fs::read_to_string(tree.join("conf/default/disable_ipv6")) {
                Ok(value) if value.trim() == "1" => Ok(()),
                Ok(_) => Err(io::Error::other(
                    "missing TUN IPv6 sysctl without disabled inheritance",
                )),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
