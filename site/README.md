# Couch landing page

Static GitHub Pages site; publish this directory as the artifact root. No build step or third-party JavaScript dependencies. `demo.js` implements the interactive preview with local sample state. All URLs are relative so project Pages paths work.

Preview from the repository root with `python3 -m http.server 8098 --directory site`.

The palette and typography follow `docs/mockups/webos-activity/`: warm paper `#eeeae5`, ink `#25232c`, muted text `#68626f`, and purple `#c4a2ff`. The Couch wordmark uses Inter at weight 800 and letter spacing −2px. The flat HTML/CSS home-screen example follows `ui/couch-gui/ui/screens/home_hub.slint` and `components/status_bar.slint`: activity strip, room cards, area dots, scenes row. All names and counts are generic example data; no personal device configuration or screenshots are published.

Inter is bundled under the SIL Open Font License; see `assets/INTER-LICENSE.txt`. Icons are unmodified SVGs copied from `assets/lucide/` in the repository and colored with CSS masks. Their ISC/Feather MIT notices are included in `site/assets/lucide/LICENSE`. No external artwork, analytics, fonts, or scripts are loaded. The demo never calls a device API and does not persist visitor changes. Rooms scroll; lights and brightness, room/global scenes, sample playback, and Home/Power/Back controls respond locally. Navigation honors reduced-motion preferences.

## Demo checks

With the preview server running on port 8098, run:

```sh
npm install --prefix build/site-review playwright
NODE_PATH=build/site-review/node_modules node tools/tests/site-demo.cjs
```

Set `SITE_URL` to check the published Pages URL. The checks cover scrolling,
return-position restoration, light/brightness state, room-scoped scenes,
playback, sleep/wake, Home/Back, keyboard input, reduced motion and mobile layout.
Screenshots are written to the system temporary directory.
