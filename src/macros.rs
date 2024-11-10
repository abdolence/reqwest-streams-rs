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
