// Local browser regression: all IR endpoints are mocked. Never sends IR.
// NODE_PATH=build/webui-review/node_modules node tools/tests/ir-library.cjs
const {chromium}=require('playwright');
const {spawn}=require('node:child_process');
const fs=require('node:fs');
const assert=require('node:assert/strict');
(async()=>{
 const configPath='/private/tmp/couch-ir-library-browser.json';
 let config={schema_version:1,revision:12,connections:[{id:'blaster',name:'Remote infrared',provider:{kind:'ir'}}],areas:[],rooms:[{id:'office',name:'Office',devices:[]}],activities:[],scenes:[]};
 fs.writeFileSync(configPath,JSON.stringify(config));
 const server=spawn('daemon/target/release/couch-confd',['--addr','127.0.0.1:18196','--config',configPath,'--no-auth','--www','web/couch-web/dist'],{stdio:'ignore'});
 const browser=await chromium.launch();
 try {
  const page=await browser.newPage({viewport:{width:1440,height:1000}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const writes=[];const stored={};let catalogReads=0;
  const extras=Array.from({length:5500},(_,i)=>({id:`extra-${i}`,brand:`Brand ${Math.floor(i/100)}`,device_type:"TV",model:`Model ${i}`,supported_commands:8}));
  await page.route('**/api/config',r=>r.fulfill({json:config}));
  await page.route('**/api/ir/**',r=>{
   const path=new URL(r.request().url()).pathname;const method=r.request().method();
   if(path==='/api/ir/catalog'){catalogReads++;return r.fulfill({json:{source:{name:'Fixture library',license:'CC0'},codesets:[{id:'lg-tv',brand:'LG',device_type:'TV',model:'Example TV',supported_commands:2},{id:'sony-avr',brand:'Sony',device_type:'Audio',model:'Example receiver',supported_commands:1},...extras]}});}
   if(path==='/api/ir/catalog/lg-tv')return r.fulfill({json:{commands:[{name:'Power',supported:true,code:'Power nec 4 8'},{name:'On',supported:true,code:'On nec 4 9'},{name:'Unsupported',supported:false,reason:'Unsupported protocol fixture'}]}});
   if(path==='/api/ir/import')return r.fulfill({json:{commands:[{name:'Imported Off',supported:true,code:'Off raw 38000 9000,4500,560,560'}]}});
   if(path.startsWith('/api/ir/codesets/')){const id=path.split('/').pop();if(method==='PUT'){stored[id]=r.request().postDataJSON().text;writes.push(stored[id]);}return r.fulfill({json:{id,text:stored[id]||'',commands:[]}});}
   throw Error('Unexpected IR request '+method+' '+path);
  });
  await page.route('**/api/rooms/office/devices**',r=>{
   assert.equal(r.request().headers()['if-match'],String(config.revision));
   assert.ok(['POST','PUT'].includes(r.request().method()));
   const device=r.request().postDataJSON();assert.equal(device.integration.resource_id,'office-tv');
   config={...config,revision:config.revision+1,rooms:[{...config.rooms[0],devices:[{id:'test-tv',...device}]}]};return r.fulfill({json:config});
  });
  await page.goto('http://127.0.0.1:18196/rooms/office');
  await page.getByLabel('IR brand',{exact:true}).selectOption('LG');
  assert.equal(await page.getByLabel('IR model',{exact:true}).locator('option').count(),2);
  await page.getByLabel('IR library device type',{exact:true}).selectOption('TV');
  await page.getByLabel('Search models',{exact:true}).fill('Example');
  await page.getByLabel('IR model',{exact:true}).selectOption('lg-tv');
  await page.getByText('Unavailable: Unsupported protocol fixture',{exact:true}).waitFor();
  await page.getByLabel('Assign Power (1)',{exact:true}).selectOption('toggle');
  await page.getByLabel('Assign On (2)',{exact:true}).selectOption('power-on');
  assert.equal(await page.getByLabel('Assigned IR commands',{exact:true}).inputValue(),'toggle nec 4 8\npower-on nec 4 9');
  await page.getByLabel('IR device name',{exact:true}).fill('Office TV');
  await page.getByText('Advanced: shared codeset',{exact:true}).click();
  assert.match(await page.getByLabel('Saved codeset ID',{exact:true}).inputValue(),/^ir-[0-9a-f]{32}$/);
  await page.getByLabel('Saved codeset ID',{exact:true}).fill('../invalid');
  assert.equal(await page.getByRole('button',{name:'Save and add to room',exact:true}).isDisabled(),true);
  await page.getByLabel('Saved codeset ID',{exact:true}).fill('office-tv');
  await page.getByText('Import your own remote codes',{exact:true}).click();
  await page.getByLabel('IR import contents',{exact:true}).fill('Filetype: IR signals file');
  await page.getByRole('button',{name:'Preview imported commands',exact:true}).click();
  await page.getByLabel('Assign Imported Off (1)',{exact:true}).selectOption('power-off');
  assert.match(await page.getByLabel('Assigned IR commands',{exact:true}).inputValue(),/power-off raw 38000 9000,4500,560,560/);
  assert.equal(writes.length,0,'Preview and assignment must not save or transmit');
  await page.setViewportSize({width:390,height:844});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.screenshot({path:'/private/tmp/couch-ir-library-mobile.png',fullPage:true});
  await page.getByRole('button',{name:'Save and add to room',exact:true}).click();
  await page.getByRole('heading',{name:'Office TV',exact:true}).waitFor();
  assert.equal(writes.length,1);assert.match(writes[0],/toggle nec 4 8/);
  const editor=page.locator('section.card').filter({has:page.getByRole('heading',{name:'Infrared commands',exact:true})});
  await editor.getByLabel('Assigned IR commands',{exact:true}).waitFor();
  await page.waitForFunction(()=>[...document.querySelectorAll('textarea[aria-label="Assigned IR commands"]')].some(e=>e.value.includes('toggle nec 4 8')));
  await editor.getByLabel('Assigned IR commands',{exact:true}).fill('toggle nec 4 10');
  await editor.getByRole('button',{name:'Save IR commands',exact:true}).click();
  await page.waitForFunction(()=>document.body.textContent.includes('Office TV'));
  for(let i=0;i<30&&config.revision<14;i++)await page.waitForTimeout(10);
  assert.equal(writes[1],'toggle nec 4 10');
  assert.equal(config.revision,14);
  assert.equal(catalogReads,1,"Room editor rerenders must reuse the catalog metadata");

  config={...config,connections:[{id:'lg',name:'LG TV',provider:{kind:'web-os'}}],rooms:[]};
  let powerSave;
  await page.route('**/api/connections/lg/webos/*',r=>{
   const op=new URL(r.request().url()).pathname.split('/').pop();
   if(op==='connection')return r.fulfill({json:{paired:true,url:'wss://192.0.2.10:3001'}});
   if(op==='power'){if(r.request().method()==='PUT')powerSave=r.request().postDataJSON();return r.fulfill({json:{method:'ir',codeset:'',blaster_available:false}});}
   throw Error('Unexpected TV control request '+op);
  });
  await page.goto('http://127.0.0.1:18196/connections');
  await page.getByText('Choose power codes from the library',{exact:true}).click();
  await page.getByLabel('IR brand',{exact:true}).selectOption('LG');
  await page.getByLabel('IR model',{exact:true}).selectOption('lg-tv');
  await page.getByLabel('Assign Power (1)',{exact:true}).waitFor();
  assert.equal(await page.getByLabel('Assign Power (1)',{exact:true}).locator('option').count(),4);
  await page.getByLabel('Assign Power (1)',{exact:true}).selectOption('power');
  await page.getByLabel('Assign On (2)',{exact:true}).selectOption('power-on');
  await page.getByRole('button',{name:'Save power settings',exact:true}).click();
  await page.getByText('Power settings saved. No command was sent.',{exact:true}).waitFor();
  assert.deepEqual(powerSave,{method:'ir',codeset:'power nec 4 8\npower-on nec 4 9'});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  assert.deepEqual(errors,[]);
  console.log('PASS: brand/type/model filters, unsupported commands, canonical assignments, raw import preview, safe IDs, revisioned room save and mobile layout; IR endpoints mocked.');
 }finally{await browser.close();server.kill();}
})().catch(e=>{console.error(e);process.exit(1)});
