# repolint

Audits a repo directory for real OSS hygiene — not "is there a file
called LICENSE" but "does the LICENSE file's actual text match what the
manifest says the license is." I wanted this because I've hit the
opposite of the obvious failure mode more than once: a repo where
`Cargo.toml` or `package.json` says `MIT` and someone pasted in the
Apache-2.0 text (or vice versa) when scaffolding the project, and it
just sits there wrong until a legal/compliance pass — or a downstream
consumer's own `licenselint`-style tool — catches it.

## Installation

```sh
cargo install --path .
```

## Usage

```bash
repolint .                       # audit the current directory
repolint path/to/some/repo
repolint . --json                # machine-readable output for CI
```

Exit code `0` if every check passes, `1` if anything failed, `2` if the
given path isn't a directory at all.

## What it checks

- **`license_present`** — a `LICENSE`, `LICENSE.md`, `LICENSE.txt`, or
  `COPYING` file exists at the repo root.
- **`license_matches_manifest`** — the LICENSE file's actual text is
  recognized as one of a fixed set of common license texts (MIT,
  Apache-2.0, GPL-2.0/3.0, LGPL-2.1/3.0, BSD-2/3-Clause, ISC, MPL-2.0,
  Unlicense) *and* that matches what `Cargo.toml`'s or `package.json`'s
  `license` field declares. If there's no manifest (or no `license`
  field) to compare against, or the LICENSE text isn't one of the
  recognized ones, this comes back `WARN` rather than `FAIL` — it
  genuinely couldn't be checked either way, which is a different thing
  from "checked, and it's wrong."
- **`readme_present`** — `README.md` (or the `README.MD`/`Readme.md`/
  bare-`README` spelling variants) exists.
- **`readme_has_sections`** — the README has a `#`-style Markdown heading
  recognizable as an Install section (`Install`, `Installation`,
  `Getting Started`, `Quick Start`) *and* one recognizable as a Usage
  section (`Usage`, `Getting Started`, `Quick Start`, `Example`).
- **`ci_config_present`** — at least one `*.yml`/`*.yaml` file under
  `.github/workflows/`, or a `.gitlab-ci.yml` at the repo root.

License detection matches on the distinctive wording every real copy of
that license carries (e.g. "Neither the name of the copyright holder"
for BSD-3 vs. plain BSD-2, "Version 3" inside "GNU GENERAL PUBLIC
LICENSE" for GPL-3 vs. GPL-2), checked against a whitespace-collapsed
copy of the file so a phrase split across a hard-wrapped line still
matches. It's a fixed set, not a general SPDX-license classifier — CC0,
WTFPL, and custom licenses correctly come back "couldn't identify"
rather than a guess.

The license-expression comparison handles a bare id (`MIT`), Cargo's
legacy slash form (`MIT/Apache-2.0`), and an `OR`/`AND`-separated
expression (`MIT OR Apache-2.0`), normalizing away the `-only`/
`-or-later` GPL suffixes. It does **not** actually distinguish `OR` from
`AND` — `MIT AND Apache-2.0` (both terms apply) and `MIT OR Apache-2.0`
(either one satisfies you) are treated identically, which is fine for
the common single-license or dual-permissive-OR case this monorepo
itself uses, and wrong for a license expression that pairs a permissive
grant with a genuinely required additional term. A full boolean SPDX
expression evaluator (see `licenselint`'s README for what that actually
takes) was out of scope here.

## Status: built, and live-verified against real sibling tools in this monorepo plus real broken fixtures

- **22 unit tests** (`cargo test --lib`): every recognized license text
  detected correctly, including BSD-3 vs. BSD-2 disambiguation via the
  endorsement clause and GPL-3 vs. GPL-2 via the version line; an
  unrecognized license text correctly returning "unknown" instead of a
  guess; the license-expression matcher against a bare id, an `OR`
  expression, Cargo's legacy slash form, the `-only`/`-or-later` suffix
  normalization, and case-insensitivity; `Cargo.toml` and both
  `package.json` license shapes (string and the older `{"type": ...}`
  object) parsed correctly; README heading detection for both the
  present and absent case, plus the `Installation`/`Getting Started`
  spelling variants; and the full `audit_repo` pipeline against
  real-shaped scratch directories — a missing LICENSE flagged, a real
  MIT-vs-Apache-2.0 mismatch flagged with both sides named in the
  detail, and a fully compliant layout passing clean.
- **Live-tested against four real sibling tools in this monorepo**
  (`dnscheck`, `leakscan`, `gitbloat`, `commitguard`), read-only. This
  caught a real bug in `detect_license_type` before it ever reached a
  test fixture: the initial version matched a single unbroken phrase
  like `"...obtaining a copy of this software..."`, but every one of
  these tools' *real* MIT `LICENSE` files (copied verbatim from
  `dnscheck/LICENSE`, itself hard-wrapped at ~79 columns) actually reads
  `"...obtaining a copy\nof this software..."` — the phrase is split
  across the wrap. Detection came back `None` on every single real MIT
  license in this workspace until the match was changed to run against
  a whitespace-collapsed copy of the text instead. After the fix, all
  four sibling tools' `license_present` and `license_matches_manifest`
  checks passed correctly against their real `Cargo.toml`+`LICENSE`
  pairs.
- **A real, unplanned finding from that same live run**: all four
  sibling tools failed `readme_has_sections` — not a bug in `repolint`,
  but a genuine fact about this monorepo's own documentation
  convention. I checked all ~55 tool READMEs in this workspace and
  **not one** has a dedicated `## Install`/`## Installation` heading;
  they go straight into `## Usage`, since these are Rust crates built
  from within the monorepo rather than something you'd
  `cargo install`/`pip install` standalone. That's a legitimate,
  reproducible gap this tool is correctly designed to catch for a repo
  that's meant to stand alone — it's just not one any current sibling
  tool here happens to close, which is exactly the kind of thing
  `repolint` is for.
- **A hand-built fully-compliant fixture** (`Cargo.toml` declaring
  `license = "MIT"`, a real verbatim MIT `LICENSE`, a `README.md` with
  actual `## Install` and `## Usage` headings, and a
  `.github/workflows/ci.yml`) passed all five checks with exit code 0 —
  confirming the tool doesn't just find problems, it recognizes their
  absence too.
- **Two hand-built broken fixtures**, each in its own scratch directory:
  one missing a `LICENSE` file entirely (correctly flagged
  `license_present: FAIL`, and, since it also had no CI config,
  `ci_config_present: FAIL` too, exit code 1); one where `Cargo.toml`
  declares `license = "MIT"` but the `LICENSE` file is the real,
  complete Apache-2.0 license text (correctly flagged
  `license_matches_manifest: FAIL` with the detail naming both sides:
  `"Cargo.toml declares \`MIT\` but the LICENSE file's actual text is
  Apache-2.0 — mismatch"`, exit code 1). All three scratch directories
  were deleted after the run.

**Not done / deliberately deferred**: full SPDX boolean-expression
evaluation (`AND` vs. `OR` — see above); recognizing non-ATX Markdown
headings (`Usage\n-----` underline style) for the README section check;
walking into subdirectories for a monorepo-style `LICENSE` that lives
somewhere other than the root; and any notion of license *compatibility*
checking against dependencies (that's `licenselint`'s job, a different
tool in this same workspace, not reimplemented here).
