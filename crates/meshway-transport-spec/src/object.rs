pub(crate) use meshway_serde::MapOnly;

macro_rules! object {
    ($($declaration:tt)*) => {
        meshway_serde::object! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use object;
