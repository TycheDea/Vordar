# Character source models

Original KayKit Adventurers characters (CC0 1.0, by Kay Lousberg —
https://kaylousberg.itch.io/kaykit-adventurers). Each GLB carries the full
76-clip animation set and unskinned gear attachments parented to bones.

These are the inputs to the preprocessing step that produces the runtime
models in `content/models/{human,dwarf,elf,valkyrie}.glb` (gear rigid-skinned
to its bone, clip set trimmed, armature scale/ground baked onto the Rig).

Regenerate with:

```
cd scripts/preprocess-characters
npm ci
node preprocess.mjs
```
