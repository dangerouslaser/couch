# Site usage guides

`site/usage/` contains the public “Using Couch” guides. Link new guides from its
index and the landing page. Describe the default physical controls separately
from activity overrides; check `ui/couch-gui/src/main.rs`, `activity_buttons.rs`,
`activity.rs`, and the relevant Slint screen before changing mapping prose.

## One renderer

The landing-page demo and guide images both use `preview/`, which compiles
`ui/couch-gui/ui/app.slint` and its production components to WebAssembly.
Do not redraw device screens in HTML, SVG, or an image editor. Add a named local
fixture to `documentation_screen` in `preview/src/demo.rs` and its expected state
to `tools/tests/site-usage.cjs`. Fixtures may supply example content and stop the
playback/toast clocks; layout, fonts, icons and rendering stay in Slint.

Each screenshot uses a fresh browser page at the remote's native 480 × 800 size.
The fixtures contain no device connections, private configuration or credentials.
Media artwork uses the same credited assets as the landing-page demo.

## Build and check

```sh
tools/build-preview.sh
python3 -m http.server 8098 --directory site
# In another terminal, using the project's local Playwright installation:
NODE_PATH=build/webui-review/node_modules node tools/tests/site-usage.cjs
NODE_PATH=build/webui-review/node_modules node tools/tests/site-demo.cjs
```

Alternatively install Playwright 1.57.0 into a temporary prefix and set
`NODE_PATH` to its `node_modules`; install its Chromium browser too.
`site-usage.cjs` generates six screenshots plus a SHA-256/source-commit inventory
in `site/usage/screenshots/`, then checks every guide at desktop and mobile widths.
Generated images are ignored by Git. Do not commit manual replacements.

The Pages workflow builds WASM, checks the landing demo, regenerates screenshots,
and checks the guides before uploading the same site artifact. GUI source,
fixture, site, button model and generator changes trigger this workflow on main.
A failed render or guide check prevents publication. The build record is linked
in each guide's footer; local dirty builds are previews, not release provenance.

After publication, verify without regenerating local images:

```sh
SITE_URL=https://dangerouslaser.github.io/couch/ \
  NODE_PATH=build/webui-review/node_modules node tools/tests/site-usage.cjs --check-only
```

Screenshots update automatically; explanations of button behavior still require
human review when mappings change. A rendered fixture checks UI output, not live
hardware, integration behavior, or the configured mappings on a user's remote.
