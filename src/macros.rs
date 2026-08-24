macro_rules! cfg_arrow {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "arrow")]
            #[cfg_attr(docsrs, doc(cfg(feature = "arrow")))]
            $item
        )*
    }
}

macro_rules! cfg_json {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "json")]
            #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
            $item
        )*
    }
}

macro_rules! cfg_csv {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "csv")]
            #[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
            $item
        )*
    }
}

macro_rules! cfg_protobuf {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "protobuf")]
            #[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
            $item
        )*
    }
}

/// Applies to items that exist only to serve the streaming formats.
///
/// Everything behind this is private machinery driven by the format modules, so with no format
/// feature enabled there is nothing to drive it and it would only warn as dead code.
macro_rules! cfg_formats {
    ($($item:item)*) => {
        $(
            #[cfg(any(
                feature = "json",
                feature = "csv",
                feature = "protobuf",
                feature = "arrow"
            ))]
            $item
        )*
    }
}
