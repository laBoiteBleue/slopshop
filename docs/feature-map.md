# Feature map

Users who come to SlopShop often know Photoshop, the most widely used professional editor, so
its menus, tools and panels serve as the reference list of features here. This page and its
table say, for each of them, what SlopShop does: implemented, planned, open to contributions,
covered differently, or left out, and why.

SlopShop does not try to copy Photoshop feature for feature. Photoshop carries thirty years of
history: some features are dated, some belong to Adobe's services, and many of them change
pixels in place where SlopShop keeps the original intact.

**The table:** [feature-map.csv](feature-map.csv) (GitHub shows it as a searchable table). It
lists menus, tools, workspaces, panels, preferences, extensibility, file formats and recent AI
features: 843 entries, named as Photoshop names them.

> SlopShop is an independent project, not affiliated with, endorsed by or sponsored by Adobe.
> Adobe and Photoshop are either registered trademarks or trademarks of Adobe in the United
> States and/or other countries. Their names are used here only to identify the features
> being compared.

## Columns

| Column      | Meaning                                                                      |
| ----------- | ---------------------------------------------------------------------------- |
| `area`      | Menu, Tool, Workspace, Panel, Preferences, Extensibility, Format, AI         |
| `reference` | Where the feature lives in Photoshop (menu path, tool group, …)              |
| `platform`  | Set only when the feature exists on one platform                             |
| `status`    | What SlopShop does with it (below)                                           |
| `slopshop`  | How SlopShop does it or will do it, what it waits for, or why it is left out |

## Statuses

| Status    | Meaning                                                                                                                                                                          |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `done`    | Exists in SlopShop today.                                                                                                                                                        |
| `partial` | Exists in part; the note says what is missing.                                                                                                                                   |
| `planned` | On the [roadmap](roadmap.md).                                                                                                                                                    |
| `open`    | Wanted, not scheduled: contributions welcome. The note says what it depends on.                                                                                                  |
| `covered` | Not as such: the need is met differently, usually by non-destructive design (an adjustment layer instead of a destructive adjustment, a layer's opacity instead of Edit > Fade). |
| `wont`    | Not planned: dated, tied to Adobe's services, or out of scope (3D, video). The note says why.                                                                                    |

A few rules decide the statuses:

- **Destructive commands become non-destructive features.** Image > Adjustments and filters
  are entries of a layer's stack, editable again, or adjustment layers; brushes, erasers and
  retouching tools paint above the original pixels, which stay intact. Commands that do rewrite
  pixels (Merge, Flatten, Rasterize) are explicit and opt-in (Layer > Bake to Pixels).
- **Dated features are left out** when a modern one covers them (Save for Web, the fixed
  Blur More and Sharpen More kernels, the Filter Gallery, slices).
- **Adobe's services are left out** (Stock, Libraries, Fonts, cloud documents, accounts).
- **3D and video are out of scope for now.** Adobe retired Photoshop's 3D features.
- **AI features are local and optional**: the AI selection tools run their models on this
  computer, and generative features will be non-destructive AI nodes; see
  [the HD generative AI research](research/hd-generative-ai.md).

## Contributing with this table

An `open` row is a candidate contribution. Selections, painting and retouching, filters,
layer styles and transforms exist, so many rows build on them; others depend on an engine
feature that does not exist yet (vector content: text, paths and shapes; patterns; a CMYK
color model; scripting): the note says so, and those rows wait for it. The rows marked "good
first issue" do not. Before starting anything larger than a small command, open an issue so
that the approach can be agreed first; see [CONTRIBUTING.md](../CONTRIBUTING.md).

Keep the table honest: a row becomes `done` only when the feature is implemented, in the same
pull request. A status change from `wont` to anything else, or the reverse, is a product
decision for the maintainer: propose it in an issue.

File formats have their own, more detailed page: [formats.md](formats.md).
