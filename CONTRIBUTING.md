# Contributing to ConvertX

Thanks for wanting to make this better. Everything here is welcome: a bug report with a sample
file, a one-line documentation fix, a new converter, or a whole new built-in engine.

This is a native desktop rewrite of [ConvertX](https://github.com/c4illin/ConvertX). If your
change belongs in the upstream web app instead (or as well), please consider sending it there too.

## Ways to help

| You want to… | Do this |
| --- | --- |
| Report something broken | Open a [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) — include the input file type, the target and the error text |
| Ask for a converter | Open a [converter request](.github/ISSUE_TEMPLATE/converter_request.yml) |
| Fix a typo or improve docs | Open a pull request directly, no issue needed |
| Add support for a new tool | Read [Adding an external converter](#adding-an-external-converter) |
| Add offline support for a format | Read [Adding a built-in engine](#adding-a-built-in-engine) |
| Test on your platform | Run the suite on Windows, Linux or an older macOS and report what breaks |

New contributors: issues labelled [`good first issue`](https://github.com/drosendo/convertx-native/labels/good%20first%20issue)
are scoped to be finished in one sitting.

## Development setup

You need Rust 1.85+ (edition 2024).

```bash
git clone https://github.com/drosendo/convertx-native
cd convertx-native
cargo run --release          # or: ./tools/package_macos.sh && open dist/ConvertX.app
```

Optional but useful: install at least one external converter so the integration tests do real work
(`brew install imagemagick ffmpeg` / `sudo apt install imagemagick ffmpeg poppler-utils`).

Linux needs windowing headers before it will build:

```bash
sudo apt install libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev \
                 libxi-dev libxrandr-dev libgl1-mesa-dev
```

## Before you open a pull request

CI runs these three on Linux, macOS and Windows — run them locally first:

```bash
cargo fmt --all                              # formatting is enforced
cargo clippy --all-targets -- -D warnings    # warnings are errors
cargo test --all-targets                     # 34 tests today
```

Guidelines that keep review quick:

- One topic per pull request. Do not mix an unrelated refactor with a fix.
- Add a test for behaviour you change. Converter work should convert a real file in a temp folder.
- Do not soften an existing test to make it pass — fix the code or explain why the test was wrong.
- Keep comments for *why*, not *what*. The code already says what it does.
- Match the surrounding style; prefer small functions and explicit error messages a user can act on.

## Adding an external converter

External converters are declared in [`assets/native.json`](assets/native.json) and get an argv
builder in [`src/convert.rs`](src/convert.rs).

1. Add a spec:

   ```json
   {
     "id": "my-tool",
     "name": "My Tool",
     "description": "One line shown in Settings.",
     "priority": 40,
     "binaries": ["mytool"],
     "version_args": ["--version"],
     "install": {
       "brew": ["install", "mytool"],
       "apt": ["install", "-y", "mytool"],
       "winget": ["install", "--id", "Vendor.MyTool"]
     },
     "from": { "images": ["png"] },
     "to":   { "images": ["pdf"] }
   }
   ```

   - `priority` decides preference: lower wins. Built-in engines sit at 900+, macOS helpers at 145,
     and the upstream converters keep their original 0–21.
   - `from` and `to` are keyed by **category**, and a route only exists inside one category.
     Reusing an existing category (`images`, `text`, `calc`, `email`, `contacts`, `muxer`,
     `object`) is almost always right.
   - Add `"platforms": ["macos"]` if the tool only exists on one OS.

2. Add the command in `build_command()`:

   ```rust
   "my-tool" => {
       let mut cmd = tools.command("mytool");
       cmd.arg(&input).arg(&target);
       (cmd, OutputMode::Direct)
   }
   ```

   Use `OutputMode::Stdout` when the tool writes to stdout, and `OutputMode::OutDir` when it derives
   the file name itself. Multi-page output that lands as `name-1.png`, `name-2.png`… is handled for you.

3. Add a test. Skip it gracefully when the tool is missing, like
   `converts_png_to_jpeg_with_imagemagick_when_available` does.

4. Mention the tool and its licence in [CREDITS.md](CREDITS.md).

You do **not** need to touch the GUI: the format picker, Settings list and install prompt read from
the registry.

## Adding a built-in engine

Built-in engines work with nothing installed. Put the code in [`src/native/`](src/native), expose
`pub fn convert(request: &ConvertRequest, started: Instant) -> ConvertResult`, register the id in
`src/native/mod.rs`, and add the spec to `assets/native.json` with `"builtin": true`.

Rules that matter:

- Never panic. Return `failure("message a user can act on", started)` instead.
- Write to `request.output` only.
- Converters run on worker threads and must be `Send`.
- A single input file must produce a single output file; pages go to `name-1.ext`, `name-2.ext`.

## Where things live

```
src/app.rs        egui interface
src/cli.rs        headless mode (--to / --optimize)
src/convert.rs    external converter argv, fallback chain, output validation
src/native/       built-in Rust engines
src/optimize.rs   the image optimizer
src/registry.rs   format routing and alias folding
src/tools.rs      tool discovery and install commands
assets/           formats.json (from upstream) and native.json (this project)
tools/gen_formats.py  regenerates formats.json from an upstream checkout
```

## Commits and pull requests

- Write the commit subject in the imperative mood: *Add HEIC to the sips converter*.
- Explain the user-visible effect in the body if it is not obvious.
- Reference the issue (`Fixes #12`) when there is one.
- Keep the pull request description short: what changed, why, and how you tested it.

## Licence

This project is **AGPL-3.0**, inherited from upstream ConvertX. By opening a pull request you agree
that your contribution is licensed under the same terms. Only submit code you have the right to
contribute, and do not paste code from projects with an incompatible licence.

## Code of conduct

Participation is covered by the [Code of Conduct](CODE_OF_CONDUCT.md). Be kind, assume good faith,
and critique the code rather than the person.
