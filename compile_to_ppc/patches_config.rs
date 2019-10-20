
mod _rel_config {
    use serde::{Serialize, Deserialize};
    #[derive(Serialize, Deserialize)]
    #[repr(C)]
    pub(crate) struct RelConfig
    {
        pub quickplay_mlvl: u32,
        pub quickplay_mrea: u32,
    }
}
pub(crate) use self::_rel_config::RelConfig;

// Thu, 17 Oct 2019 12:53:11 GMT
