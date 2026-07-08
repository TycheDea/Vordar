// Turns the KayKit Adventurers sources (content/source/characters/*.glb) into
// the runtime character models (content/models/{race}.glb):
//   - keeps the skinned body meshes plus one gear loadout per race, rigid-binding
//     each gear mesh to its parent bone so it rides the animation through the
//     ordinary skinned pipeline (the engine's glTF loader would otherwise bake
//     unskinned nodes at bind pose, detached from the rig);
//   - trims the 76-clip library down to the clips the game actually plays;
//   - bakes a uniform world scale + ground offset onto the Rig node, which the
//     engine folds into every root joint (Skeleton::root).
// Run: npm ci && node preprocess.mjs   (from scripts/preprocess-characters/)
import { NodeIO } from '@gltf-transform/core';
import { prune, dedup } from '@gltf-transform/functions';
import { statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.join(HERE, '..', '..', 'content', 'source', 'characters');
const OUT = path.join(HERE, '..', '..', 'content', 'models');

const io = new NodeIO();
const TARGET_H = 1.7;   // world-space character height
const FLOOR_TOP = -0.5; // y of the floor surface the feet stand on
const EXPECTED_JOINTS = 41;
const MAX_OUT_BYTES = 3.5 * 1024 * 1024;

// Clips kept per character. Locomotion (idle/walk/run), the per-ability attack
// clips mapped in content/classes/*.ron, hit react, and death.
const KEEP_CLIPS = [
  'Idle',
  'Walking_A',
  'Running_A',
  '1H_Melee_Attack_Chop',
  '1H_Melee_Attack_Slice_Horizontal',
  '2H_Melee_Attack_Spinning',
  'Spellcast_Shoot',
  'Spellcast_Long',
  'Hit_A',
  'Death_A',
];

// race -> { char, gear: [node names to keep + rigid-bind to their parent bone] }
// Everything else unskinned (capes, alternate weapons, mugs) is dropped.
const PLAN = {
  human:    { char: 'Knight',    gear: ['1H_Sword', 'Round_Shield', 'Knight_Helmet'] },
  dwarf:    { char: 'Barbarian', gear: ['1H_Axe', 'Barbarian_Round_Shield', 'Barbarian_Hat'] },
  elf:      { char: 'Rogue',     gear: ['Knife', 'Knife_Offhand'] },
  valkyrie: { char: 'Mage',      gear: ['2H_Staff', 'Mage_Hat'] },
};

// Column-major 4x4 helpers (gltf-transform getMatrix() is column-major number[16]).
const I4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
function mul4(a, b) { // a*b
  const o = new Array(16).fill(0);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
    let s = 0;
    for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
    o[c * 4 + r] = s;
  }
  return o;
}
function mulVec(m, v) { // m * [x,y,z,w]
  return [0, 1, 2, 3].map(r => m[0 * 4 + r] * v[0] + m[1 * 4 + r] * v[1] + m[2 * 4 + r] * v[2] + m[3 * 4 + r] * v[3]);
}
function mulVec3Dir(m, v) { // upper 3x3 * dir (normals; fine for rot + uniform scale)
  return [0, 1, 2].map(r => m[0 * 4 + r] * v[0] + m[1 * 4 + r] * v[1] + m[2 * 4 + r] * v[2]);
}
function norm3(v) { const l = Math.hypot(v[0], v[1], v[2]) || 1; return [v[0] / l, v[1] / l, v[2] / l]; }

function worldMatrix(node, parentOf) {
  const chain = [];
  let cur = node;
  while (cur) { chain.push(cur); cur = parentOf.get(cur); }
  chain.reverse();
  let W = I4;
  for (const n of chain) W = mul4(W, n.getMatrix());
  return W;
}

function assert(cond, msg) {
  if (!cond) { console.error(`ASSERT FAILED: ${msg}`); process.exit(1); }
}

for (const [race, spec] of Object.entries(PLAN)) {
  const doc = await io.read(path.join(SRC, `${spec.char}.glb`));
  const root = doc.getRoot();
  const buffer = root.listBuffers()[0];
  const skin = root.listSkins()[0];
  const joints = skin.listJoints();
  assert(joints.length === EXPECTED_JOINTS, `${race}: expected ${EXPECTED_JOINTS} joints, got ${joints.length}`);
  const parentOf = new Map();
  for (const n of root.listNodes()) for (const c of n.listChildren()) parentOf.set(c, n);

  const gearSet = new Set(spec.gear);
  const bound = [];
  for (const node of root.listNodes()) {
    const mesh = node.getMesh();
    if (!mesh || node.getSkin()) continue;      // skinned body meshes: leave as-is
    if (!gearSet.has(node.getName())) { node.dispose(); continue; } // unwanted attachment
    gearSet.delete(node.getName());

    // Rigid-bind this gear node to its parent bone: bake the node's bind-world
    // matrix into the geometry and weight every vertex 100% to that bone. The
    // inverse-bind matrix then maps it back to bone space, so it poses with
    // the bone exactly like the body (palette = global * inverse_bind).
    const bone = parentOf.get(node);
    const j = joints.indexOf(bone);
    assert(j >= 0, `${race}: ${node.getName()} parent ${bone?.getName()} is not a skin joint`);
    const W = worldMatrix(node, parentOf);

    for (const prim of mesh.listPrimitives()) {
      const pos = prim.getAttribute('POSITION');
      const nrm = prim.getAttribute('NORMAL');
      const count = pos.getCount();
      const pa = pos.getArray().slice();
      for (let i = 0; i < count; i++) {
        const w = mulVec(W, [pa[i * 3], pa[i * 3 + 1], pa[i * 3 + 2], 1]);
        pa[i * 3] = w[0]; pa[i * 3 + 1] = w[1]; pa[i * 3 + 2] = w[2];
      }
      pos.setArray(pa);
      if (nrm) {
        const na = nrm.getArray().slice();
        for (let i = 0; i < count; i++) {
          const n = norm3(mulVec3Dir(W, [na[i * 3], na[i * 3 + 1], na[i * 3 + 2]]));
          na[i * 3] = n[0]; na[i * 3 + 1] = n[1]; na[i * 3 + 2] = n[2];
        }
        nrm.setArray(na);
      }
      const jArr = new Uint8Array(count * 4);
      const wArr = new Float32Array(count * 4);
      for (let i = 0; i < count; i++) { jArr[i * 4] = j; wArr[i * 4] = 1; }
      prim.setAttribute('JOINTS_0', doc.createAccessor().setType('VEC4').setArray(jArr).setBuffer(buffer));
      prim.setAttribute('WEIGHTS_0', doc.createAccessor().setType('VEC4').setArray(wArr).setBuffer(buffer));
    }
    node.setSkin(skin);
    node.setMatrix(I4); // geometry baked into skin space; node transform is ignored for skinned meshes
    bound.push(`${node.getName()}->${bone.getName()}(j${j})`);
  }
  assert(gearSet.size === 0, `${race}: gear nodes not found in source: ${[...gearSet].join(', ')}`);

  // Trim clips.
  const keep = new Set(KEEP_CLIPS);
  for (const a of root.listAnimations()) if (!keep.has(a.getName())) a.dispose();

  // Bake armature scale + ground onto Rig (feet from skinned body bounds).
  let maxY = -1e9, minY = 1e9;
  for (const n of root.listNodes()) {
    const m = n.getMesh();
    if (!m || !n.getSkin()) continue;
    for (const p of m.listPrimitives()) {
      const P = p.getAttribute('POSITION');
      if (!P) continue;
      maxY = Math.max(maxY, P.getMax([])[1]);
      minY = Math.min(minY, P.getMin([])[1]);
    }
  }
  const s = TARGET_H / (maxY - minY);
  const rig = root.listScenes()[0].listChildren().find(n => n.getName() === 'Rig');
  assert(rig, `${race}: no Rig node at scene root`);
  rig.setScale([s, s, s]);
  rig.setTranslation([0, FLOOR_TOP - minY * s, 0]);

  await doc.transform(prune(), dedup());

  // Verify the output before writing.
  const clipNames = root.listAnimations().map(a => a.getName()).sort();
  assert(
    JSON.stringify(clipNames) === JSON.stringify([...KEEP_CLIPS].sort()),
    `${race}: clip set mismatch: ${clipNames.join(', ')}`,
  );
  for (const n of root.listNodes()) {
    const m = n.getMesh();
    if (!m) continue;
    assert(n.getSkin(), `${race}: mesh node ${n.getName()} is not skinned after preprocessing`);
    for (const p of m.listPrimitives()) {
      assert(p.getAttribute('JOINTS_0') && p.getAttribute('WEIGHTS_0'),
        `${race}: primitive on ${n.getName()} missing skin attributes`);
    }
  }
  assert(root.listSkins()[0].listJoints().length === EXPECTED_JOINTS,
    `${race}: joint count changed after prune`);

  const outPath = path.join(OUT, `${race}.glb`);
  await io.write(outPath, doc);
  const size = statSync(outPath).size;
  assert(size < MAX_OUT_BYTES, `${race}: output ${size} bytes exceeds ${MAX_OUT_BYTES}`);
  console.log(`${race.padEnd(9)} <- ${spec.char.padEnd(10)} scale=${s.toFixed(3)} ${(size / 1e6).toFixed(2)}MB gear: ${bound.join(', ')}`);
}
console.log('OK');
