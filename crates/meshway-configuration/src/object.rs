pub(crate) use meshway_serde::unique_map;

macro_rules! object {
    ($($declaration:tt)*) => {
        meshway_serde::object! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use object;

macro_rules! tagged {
    ($($declaration:tt)*) => {
        meshway_serde::tagged! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use tagged;
