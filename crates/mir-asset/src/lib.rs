//! Miriya 资产系统：稳定 ID、注册表、依赖图、类型化缓存与异步加载（M4）。

mod asset;
mod cache;
mod container;
mod dependency;
mod error;
mod hot_reload;
mod id;
pub mod importer;
mod registry;
mod server;

pub use asset::{
    Asset, AssetState, AudioAsset, AudioData, Handle, ImageAsset, Ktx2Asset, MaterialAsset,
    MeshAsset, NativeAsset, SceneAsset,
};
pub use cache::AssetCache;
pub use container::{ContainerHeader, decode, encode, encode_with_version};
pub use dependency::DependencyGraph;
pub use error::{AssetError, Result};
pub use id::AssetId;
pub use registry::{AssetMeta, AssetRegistry};
pub use server::AssetServer;
