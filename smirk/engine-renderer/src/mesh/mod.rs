// glTF mesh rendering — the "real models" path next to the primitive pool.
//
// Three stages, split so the parse is testable without a GPU device:
//   CPU parse: load_gltf_data(path) -> MeshData   (gltf_import: parse, node
//        transforms baked into vertices, per-primitive base-color material)
//   GPU upload: MeshStore::get_or_request enqueues a background decode on
//        miss; integrate uploads completed loads into vertex/index buffers
//        and a bind group per primitive under a per-frame budget (store:
//        device/queue work)
//   Per-frame: MeshRenderSyncSystem collects drawable entities into draw lists,
//        sampling animation poses (sync: the drawable-collection system).
//
// Unlike the SdfInstance pool there is no slot bookkeeping: the draw list is
// rebuilt from live entities every frame by MeshRenderSyncSystem, so despawn
// needs no hook and instancing falls out of grouping by mesh index.

mod anim_import;
mod gltf_import;
#[cfg(test)]
mod test_glb;
mod store;
mod sync;

pub use gltf_import::{load_gltf_data, load_image_rgba, AlphaMode, ImageData, MaterialData, MeshData, PrimitiveData, SharedImage, TextureSource, VertexSkin};
pub use store::MeshStore;
pub(crate) use store::slot_texture;
#[cfg(feature = "offscreen")]
pub(crate) use store::upload_mesh;
pub use sync::{MeshDrawList, MeshRenderSyncSystem, SkinnedDrawList, SocketConfig, SocketTransforms};
