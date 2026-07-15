# Design

Adopted from open-web's DESIGN.md (Liberte web re-platform ruling, D6,
2026-07-15) as the reviewed local contract of record for `web/`. The face
answers to the same constitution as the source: a closed vocabulary,
declared territories, and violations a freshman can point at. Divergence
from the adopted constitution is named in review and returns to Liberte
for a separate ruling; this file is never silently redefined.

## The four organs

All four live in `web/src/components`, the style territory.

- `tokens.scss` — **defines** the design vocabulary. One typed `@property`
  registration per token; a var not registered here does not exist. Typed
  registrations carry canary initials (`magenta`, `0px`): a theme that
  forgets a binding confesses on screen.
- `themes/glass.scss` — **declares values**. One binding table; every
  registered token bound exactly once. This is the only file in the
  repository where a design literal (hex, rem, ms) may appear.
  Compile-time scss locals may dedupe values inside the theme; they never
  leave the file.
- `media.scss` — **defines the seams**. The only home of `@media`. The
  seam inventory is currently EMPTY: the pane is fluid-only. When the
  first seam materializes it is spelled once here as a mixin and flips a
  `--seam` marker, per the open-web original.
- `<Atom>.scss` — **consumes**. Component sheets speak `var()` plus the
  keyword whitelist below, and reference seams only via media mixins.
  No literals, no raw `@media`, ever.

## The token table

Tokens are registered on first use; an unused token is a squatter and is
evicted. The mechanical checker (`.runseal/lib/web/tokens.ts`, run by the
guard) enforces all three directions: every registered token is consumed,
every consumed token is registered, and `glass.scss` binds each registered
token exactly once.

Current seats:

| dimension | tokens |
| --- | --- |
| color | `ground` `ink` `bright` `muted` |
| type | `hero` |
| space | `space-2` `space-3` `space-5` |
| measure | `page` `prose` |
| rhythm | `leading` `tight` |
| weight | `heft` |
| face | `sans` |

The open-web 28-seat table is the candidate ceiling; a seat is claimed
only by the change that consumes it.

## The whitelist

Atom sheets may use, beyond `var()`, exactly these keyword and identity
values:

`auto` `none` `center` `flex` `grid` `inline-flex` `inline-block` `block`
`wrap` `nowrap` `pointer` `border-box` `hidden` `visible` `scroll`
`baseline` `stretch` `start` `end` `space-between` `column` `row`
`normal` `bold` `italic` `underline` `uppercase` `lowercase` `inherit`
`transparent` `currentColor` `solid` `0` `100%` `1fr` `1`

No unit literal is ever whitelisted — the moment a rule wants a number
with a unit, it wants a token. Extending this list is a reviewed contract
change: name the entry, state the rationale, land it with the change that
needs it.

## The consumption ladder

Media differences resolve at the cheapest rung that holds; each rung down
needs a harder reason.

1. **Fluid** — layout absorbs the continuum; most differences die here.
2. **Rebind** — the theme rebinds a var under a seam; atoms stay ignorant.
3. **Shift** — an atom restructures under a media mixin. Rare; each
   instance reviewable on its own.
4. **Fork** — the atom splits into media-exclusive incarnations: one
   public name, one incarnation per media world under a media-named
   directory, a single dispatcher inside the components territory reading
   the `--seam` marker. Size, density, and spacing never justify a fork.

## Naming

A token name is one vocabulary atom, optionally followed by an ordinal
(`--ink`, `--space-2`). If a token cannot be named in one word, its seat
is suspect.

## Doctrine

- Fluid first; the face gets named seams, never a responsive continuum.
- A design literal has exactly one legal residence: `themes/glass.scss`.
- `@media` is spelled in exactly one file: `media.scss`.
- Interactive incarnations mount singly; `display: none` is not a
  dispatcher.
- The pane is an audit surface: self-contained bundle, no external
  requests, ever.
