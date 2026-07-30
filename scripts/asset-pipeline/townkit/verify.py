"""Re-import a just-exported glb into a fresh scene and check it: material
slot names against the six-material vocabulary, the vordar_detail extra on
limestone materials, UV layers, loose geometry, and boundary-edge counts."""

import bmesh
import bpy

ALLOWED_MATERIALS = {
    "encalado", "limestone_dressed", "terracotta_tile",
    "oak_dark", "plaster_smoked", "iron_wrought",
}


def verify_glb(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(path))

    mesh_objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    report = {
        "mesh_count": len(mesh_objs),
        "material_names": set(),
        "bad_material_names": [],
        "detail_extras": {},
        "missing_uv": [],
        "loose_verts": 0,
        "loose_edges": 0,
        "boundary_edges": 0,
        "total_tris": 0,
    }

    for mat in bpy.data.materials:
        report["material_names"].add(mat.name)
        if mat.name not in ALLOWED_MATERIALS:
            report["bad_material_names"].append(mat.name)
        report["detail_extras"][mat.name] = mat.get("vordar_detail", None)

    for obj in mesh_objs:
        mesh = obj.data
        if not mesh.uv_layers:
            report["missing_uv"].append(obj.name)
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bm.faces.ensure_lookup_table()
        mesh.calc_loop_triangles()
        report["total_tris"] += len(mesh.loop_triangles)
        for v in bm.verts:
            if len(v.link_faces) == 0:
                report["loose_verts"] += 1
        for e in bm.edges:
            nfaces = len(e.link_faces)
            if nfaces == 0:
                report["loose_edges"] += 1
            elif nfaces == 1:
                report["boundary_edges"] += 1
        bm.free()

    report["ok"] = (not report["bad_material_names"] and not report["missing_uv"]
                     and report["loose_verts"] == 0 and report["loose_edges"] == 0)
    return report
