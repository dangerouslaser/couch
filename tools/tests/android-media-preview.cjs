// Check the production Slint canvas using local, synthetic Android TV metadata.
// Build with tools/build-preview.sh, serve site/, and set NODE_PATH to Playwright.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
const base=process.env.SITE_URL||'http://127.0.0.1:8098/';
const cases={
 'android-tv':{media_active:true,media_live:false,media_has_duration:true,media_has_art:true,media_paused:false},
 'android-paused':{media_active:true,media_paused:true},
 'android-live':{media_active:true,media_live:true},
 'android-no-duration':{media_active:true,media_live:false,media_has_duration:false},
 'android-no-art':{media_active:true,media_has_art:false},
 'android-idle':{media_active:false},
 'apple-tv':{android_tv:false,media_active:false},
 webos:{android_tv:false,media_active:false},
 infrared:{android_tv:false,media_active:false},
};
(async()=>{
 assert.ok(['localhost','127.0.0.1'].includes(new URL(base).hostname),'Use local fixtures only');
 const browser=await chromium.launch();
 try{
  const page=await browser.newPage({viewport:{width:480,height:800},deviceScaleFactor:1});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(new URL('preview.html',base).href);
  await page.waitForFunction(()=>!!window.couchDemo?.documentation_screen);
  const fixture=async(name,expected)=>{
   await page.evaluate(n=>window.couchDemo.documentation_screen(n),name);
   await page.waitForFunction(e=>Object.entries(e).every(([k,v])=>window.couchDemo.state()[k]===v),{tv:true,tv_panel:0,...expected});
   await page.waitForTimeout(500);
  };
  for(const [name,expected] of Object.entries(cases)){
   await fixture(name,expected);
   if(process.env.SCREENSHOT_DIR)await page.locator('canvas').screenshot({path:`${process.env.SCREENSHOT_DIR}/${name}.png`});
  }
  await fixture('android-tv',cases['android-tv']);
  await page.mouse.click(290,584); // Pause uses the normal TV action callback.
  await page.waitForFunction(()=>window.couchDemo.state().media_paused);
  await page.mouse.click(188,584);
  await page.waitForFunction(()=>!window.couchDemo.state().media_paused);
  await page.mouse.click(375,687); // Existing shared Apps tray.
  await page.waitForFunction(()=>window.couchDemo.state().tv_panel===2);
  await page.waitForTimeout(500);
  if(process.env.SCREENSHOT_DIR)await page.locator('canvas').screenshot({path:`${process.env.SCREENSHOT_DIR}/android-apps.png`});
  // The outside scrim dismisses independently of tray sizing.
  await page.mouse.click(240,120);
  await page.waitForFunction(()=>window.couchDemo.state().tv_panel===0);
  await page.evaluate(()=>window.couchDemo.remote_button('ok'));
  await page.waitForTimeout(100);
  assert.equal((await page.evaluate(()=>window.couchDemo.state())).tv,true,'Physical OK keeps TV control takeover');
  assert.deepEqual(errors,[]);
  console.log('PASS: Android media states, play/pause actions, Apps tray, physical takeover, other TV fallbacks');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
