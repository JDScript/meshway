//! RC-1 M4 (G5, G6): one classification of a failed kernel operation. Only
//! a read-back that differs from what the owner owns is a conflict; a read or
//! change that failed, timed out or stopped, an internal failure and a
//! cancelled installation prove nothing and end ownership as unavailable.
use super::super::*;

#[test]
fn only_a_differing_read_back_ends_ownership_as_a_conflict() {
    for (operation, unavailable) in [
        ("install", "install_unavailable"),
        ("repair", "repair_unavailable"),
    ] {
        for error in [
            Error::Netlink,
            Error::Io(std::io::ErrorKind::TimedOut.into()),
            Error::Worker,
        ] {
            assert_eq!(
                ended(operation, &error),
                crate::OwnershipEnd::Unavailable { check: unavailable },
                "{operation}: {error}"
            );
        }
        assert_eq!(
            ended(operation, &Error::Drain),
            crate::OwnershipEnd::Unavailable {
                check: "drain_unavailable"
            }
        );
        assert_eq!(
            ended(operation, &Error::Conflict),
            crate::OwnershipEnd::Conflict { check: operation }
        );
    }
}
