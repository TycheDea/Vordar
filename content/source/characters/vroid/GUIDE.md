# VRoid look-test guide (~15 minutes)

Goal: one semi-chibi test character to drop into the dusk start zone next to
the current knight, so we can judge Ragnarok-Online-like proportions in-engine
before locking the art direction. Don't polish — this is a proportion and
style probe, not the final character.

## 1. Install

VRoid Studio, free: https://vroid.com/en/studio (also on Steam, same app).
Characters you create are yours, commercial use included.

## 2. Create the test character

Start from any base preset (pick whichever gender reads better to you), then:

**Proportions (Body editor)** — aim for a "2.5–3 heads tall" silhouette:
- Height: pull way down (short/child-range).
- Head: as large as the sliders allow — the head should read as roughly a
  third of total height. If there's a head-to-body ratio control in your
  version, push it toward the cartoonish end.
- Arms and legs: short and slightly thick; hands can stay a touch oversized
  (reads well at our camera distance).
- Shoulders/torso: narrow — chibi torsos are small relative to the head.

Exact slider names shift between VRoid versions; the target silhouette is
what matters. Eyeball it against Ragnarok Origin screenshots if unsure.

**Hair** — use the hair editor freely; pick or edit any preset you like.
This doubles as the test of "hair as customization", so make it a hairstyle
you'd actually want in the game.

**Face/skin** — keep it simple. Slightly muted skin tone fits the dusk zone
better than bright anime pink. Avoid heavy blush/face-line textures.

**Outfit** — whatever's quickest (a default outfit is fine). For the real
pipeline later we'll want a plain base body, but the look-test just needs a
character standing in the world.

## 3. Export

Camera/Export mode → **Export as VRM**:
- VRM version: **VRM 0.0** if asked (widest tool compatibility).
- Enable polygon reduction if offered — target roughly ≤ 40k triangles
  (reduce hair the most; it's usually the heaviest).
- Texture atlas: 2048.
- "Delete transparent meshes": fine to enable.

Save as: `content/source/characters/vroid/test01.vrm`

## 4. Then tell me

I take it from there: VRM → glb conversion, drop it into the start zone next
to the KayKit knight for the side-by-side feel-check (dusk lighting, our
camera). If the proportions win, VRoid becomes the interim character source —
races and hairstyles as more VRoid exports — while the modular pipeline
(Mixamo auto-rig + clip library + skinned armor attachments) is built around
it. See tasks/character-direction-notes.md for the full plan.
