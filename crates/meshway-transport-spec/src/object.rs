pub(crate) use meshway_model::wire::MapOnly;

macro_rules! object {
    ($($declaration:tt)*) => {
        meshway_model::object! {
            #[derive(Clone, Debug, PartialEq, Eq)]
            $($declaration)*
        }
    };
}
pub(crate) use object;
