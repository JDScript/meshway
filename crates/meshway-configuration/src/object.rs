pub(crate) use meshway_model::wire::unique_map;

macro_rules! object {
    ($($declaration:tt)*) => {
        meshway_model::object! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use object;

macro_rules! tagged {
    ($($declaration:tt)*) => {
        meshway_model::tagged! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use tagged;
