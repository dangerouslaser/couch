const icons = {"arrow-left": "\n  <path d=\"m12 19-7-7 7-7\" />\n  <path d=\"M19 12H5\" />\n", "arrow-right": "\n  <path d=\"M5 12h14\" />\n  <path d=\"m12 5 7 7-7 7\" />\n", "arrow-up": "\n  <path d=\"m5 12 7-7 7 7\" />\n  <path d=\"M12 19V5\" />\n", "arrow-down": "\n  <path d=\"M12 5v14\" />\n  <path d=\"m19 12-7 7-7-7\" />\n", "battery-medium": "\n  <path d=\"M10 14v-4\" />\n  <path d=\"M22 14v-4\" />\n  <path d=\"M6 14v-4\" />\n  <rect x=\"2\" y=\"6\" width=\"16\" height=\"12\" rx=\"2\" />\n", "wifi": "\n  <path d=\"M12 20h.01\" />\n  <path d=\"M2 8.82a15 15 0 0 1 20 0\" />\n  <path d=\"M5 12.859a10 10 0 0 1 14 0\" />\n  <path d=\"M8.5 16.429a5 5 0 0 1 7 0\" />\n", "pause": "\n  <rect x=\"14\" y=\"3\" width=\"5\" height=\"18\" rx=\"1\" />\n  <rect x=\"5\" y=\"3\" width=\"5\" height=\"18\" rx=\"1\" />\n", "play": "\n  <path d=\"M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z\" />\n", "rotate-ccw": "\n  <path d=\"M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8\" />\n  <path d=\"M3 3v5h5\" />\n", "rotate-cw": "\n  <path d=\"M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8\" />\n  <path d=\"M21 3v5h-5\" />\n", "list-video": "\n  <path d=\"M21 5H3\" />\n  <path d=\"M10 12H3\" />\n  <path d=\"M10 19H3\" />\n  <path d=\"M15 12.003a1 1 0 0 1 1.517-.859l4.997 2.997a1 1 0 0 1 0 1.718l-4.997 2.997a1 1 0 0 1-1.517-.86z\" />\n", "captions": "\n  <rect width=\"18\" height=\"14\" x=\"3\" y=\"5\" rx=\"2\" ry=\"2\" />\n  <path d=\"M7 15h4M15 15h2M7 11h2M13 11h4\" />\n", "audio-lines": "\n  <path d=\"M2 10v3\" />\n  <path d=\"M6 6v11\" />\n  <path d=\"M10 3v18\" />\n  <path d=\"M14 8v7\" />\n  <path d=\"M18 5v13\" />\n  <path d=\"M22 10v3\" />\n", "gamepad-2": "\n  <line x1=\"6\" x2=\"10\" y1=\"11\" y2=\"11\" />\n  <line x1=\"8\" x2=\"8\" y1=\"9\" y2=\"13\" />\n  <line x1=\"15\" x2=\"15.01\" y1=\"12\" y2=\"12\" />\n  <line x1=\"18\" x2=\"18.01\" y1=\"10\" y2=\"10\" />\n  <path d=\"M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z\" />\n", "x": "\n  <path d=\"M18 6 6 18\" />\n  <path d=\"m6 6 12 12\" />\n", "check": "\n  <path d=\"M20 6 9 17l-5-5\" />\n", "tv": "\n  <path d=\"m17 2-5 5-5-5\" />\n  <rect width=\"20\" height=\"15\" x=\"2\" y=\"7\" rx=\"2\" />\n", "house": "\n  <path d=\"M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8\" />\n  <path d=\"M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\" />\n", "monitor-off": "\n  <path d=\"M12 17v4\" />\n  <path d=\"M17 17H4a2 2 0 0 1-2-2V5a2 2 0 0 1 1.184-1.826\" />\n  <path d=\"m2 2 20 20\" />\n  <path d=\"M8 21h8\" />\n  <path d=\"M8.656 3H20a2 2 0 0 1 2 2v10a2 2 0 0 1-.293 1.042\" />\n", "chevron-down": "\n  <path d=\"m6 9 6 6 6-6\" />\n", "ellipsis": "\n  <circle cx=\"12\" cy=\"12\" r=\"1\" />\n  <circle cx=\"19\" cy=\"12\" r=\"1\" />\n  <circle cx=\"5\" cy=\"12\" r=\"1\" />\n", "film": "\n  <rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\" />\n  <path d=\"M7 3v18\" />\n  <path d=\"M3 7.5h4\" />\n  <path d=\"M3 12h18\" />\n  <path d=\"M3 16.5h4\" />\n  <path d=\"M17 3v18\" />\n  <path d=\"M17 7.5h4\" />\n  <path d=\"M17 16.5h4\" />\n"};
const icon=n=>`<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">${icons[n]||icons.film}</svg>`;
const logo=`<svg class="clearlogo" viewBox="0 0 320 124" role="img" aria-label="Tears of Steel, mock clearlogo"><text x="2" y="30" fill="white" font-family="Inter,sans-serif" font-weight="450" font-size="23" letter-spacing="9.5">TEARS OF</text><text x="0" y="101" fill="white" font-family="Inter,sans-serif" font-weight="780" font-size="80" letter-spacing="-4">STEEL</text><path d="M4 117h270" stroke="white" stroke-opacity=".55"/></svg>`;
const concepts=[['a','Cinema','Artwork leads. The controls stay close.','RECOMMENDED'],['b','Focus','A quiet surface. A generous control deck.',''],['c','Quiet','The film comes first. Details unfold on demand.','']];
const chapters=[['Opening',0],['The promise',88],['A different future',198],['The plan',302],['One more chance',436],['Credits',626]];
const models=Object.fromEntries(concepts.map(([id])=>[id,{playing:true,position:312,total:734,fixture:'playing',sheet:null,audio:'English · 5.1',subtitles:'Off',exited:false}]));
const clock=t=>`${Math.floor(t/60)}:${String(Math.floor(t%60)).padStart(2,'0')}`;
const button=(action,ico,label,cls='',disabled=false)=>`<button type="button" data-action="${action}" class="${cls}" aria-label="${label}" ${disabled?'disabled':''}>${icon(ico)}</button>`;
const controls=(m,id)=>`<div class="playback"><div class="timing"><strong data-elapsed>${m.fixture==='live'?'LIVE':clock(m.position)}</strong><span data-remaining>${m.fixture==='live'?'Live broadcast':`−${clock(m.total-m.position)}`}</span></div><input class="seek" aria-label="Playback position" type="range" min="0" max="${m.total}" value="${m.position}" style="--played:${m.position/m.total*100}%" ${m.fixture==='live'?'disabled':''}><div class="transport">${button('rewind','rotate-ccw','Back 10 seconds','skip',m.fixture==='live').replace('</button>','<small>10</small></button>')}${button('play',m.playing?'pause':'play',m.playing?'Pause':'Play','play')}${button('forward','rotate-cw','Forward 30 seconds','skip',m.fixture==='live').replace('</button>','<small>30</small></button>')}</div><div class="actions">${button('chapters','list-video','Chapters','',m.fixture==='live').replace('</button>','<span>Chapters</span></button>')}${button('audio','audio-lines','Audio').replace('</button>','<span>Audio</span></button>')}${button('subtitles','captions','Subtitles').replace('</button>','<span>Subtitles</span></button>')}</div><div class="bottomline">${button(id==='c'?'more':'tv',id==='c'?'ellipsis':'gamepad-2',id==='c'?'More controls':'Control Kodi menus','tv-toggle').replace('</button>',`<span>${id==='c'?'More controls':'TV controls'}</span></button>`)}</div></div>`;
function sheet(m){
 if(!m.sheet)return '';
 const title={chapters:'Chapters',audio:'Audio',subtitles:'Subtitles',tv:'TV controls',more:'Playback controls'}[m.sheet];
 let inner='';
 if(m.sheet==='chapters'){
  if(m.fixture==='legacy')inner='<p>This Kodi connection does not provide a chapter list. You can still seek using the progress bar.</p>';
  else{let active=0;chapters.forEach((c,i)=>{if(m.position>=c[1])active=i});inner=`<p>Tears of Steel · 6 chapters</p><div class="chapter-list">${chapters.map(([name,time],i)=>`<button data-action="chapter:${i}" class="${i===active?'current':''}" aria-label="Chapter ${i+1}: ${name}, ${clock(time)}"><span class="num">${String(i+1).padStart(2,'0')}</span><span class="chapter-title">${name}</span><span class="time">${clock(time)}</span></button>`).join('')}</div>`}
 }
 if(m.sheet==='audio'||m.sheet==='subtitles'){
  const values=m.sheet==='audio'?['English · 5.1','English · Stereo','Director commentary']:['Off','English','English · SDH'];
  inner=`<p>${m.sheet==='audio'?'Choose an audio track.':'Choose a subtitle track.'}</p><div class="selection-list">${values.map(v=>`<button data-action="track:${v}" class="${m[m.sheet]===v?'current':''}"><span>${v}</span>${m[m.sheet]===v?icon('check'):''}</button>`).join('')}</div>`;
 }
 if(m.sheet==='tv')inner=`<p>The D-pad now controls Kodi on your TV.<br>Back closes these controls.</p><div class="dpad">${[['Up','arrow-up'],['Left','arrow-left'],['Select','check'],['Right','arrow-right'],['Down','arrow-down']].map(([v,i])=>button('kodi:'+v,i,v)).join('')}</div><div class="tv-bottom">${button('kodi:Back','arrow-left','Back on TV').replace('</button>','Back on TV</button>')}${button('kodi:Home','house','Kodi home').replace('</button>','Kodi home</button>')}</div>`;
 if(m.sheet==='more')inner=`<p>Keep the picture clear. Everything else is here.</p><div class="selection-list">${[['chapters','Chapters'],['audio','Audio'],['subtitles','Subtitles'],['tv','TV controls']].map(([a,t])=>`<button data-action="${a}" ${a==='chapters'&&m.fixture==='live'?'disabled':''}>${t}${icon('arrow-right')}</button>`).join('')}</div>`;
 return `<div class="sheet-scrim" data-action="close"></div><section class="sheet" role="dialog" aria-modal="true" aria-label="${title}"><div class="sheet-head"><h3>${title}</h3>${button('close','x','Close panel')}</div>${inner}</section>`;
}
function render(id,focusAction){
 const m=models[id],node=document.getElementById('remote-'+id);
 const dead=m.fixture==='idle'||m.fixture==='offline';
 node.className=`remote ${id} ${m.fixture==='missing'?'no-art':''}`;
 node.innerHTML=`<div class="fanart"></div><div class="shade"></div><div class="top">${button('exit','arrow-left','Return to room','exit')}<div class="room-label">Watch Kodi<small>Living room</small></div><div class="status">${icon('wifi')}${icon('battery-medium')}</div></div><div class="brand-art">${m.fixture==='missing'?'<h3 class="fallback-title">Tears of Steel</h3>':logo}<div class="movie-meta"><span>2012</span><span>Sci-fi</span><span>12 min</span></div></div>${id==='b'?`<div class="state-label">${m.playing?'NOW PLAYING':'PAUSED'}</div>`:''}${controls(m,id)}`;
 if(dead){node.innerHTML=`<div class="empty-state">${icon(m.fixture==='offline'?'monitor-off':'tv')}<h3>${m.fixture==='offline'?'Kodi is unavailable':'Ready when you are.'}</h3><p>${m.fixture==='offline'?'Check that your media player is on and connected.':'Choose something on your TV.<br>Playback controls will appear here.'}</p>${button(m.fixture==='offline'?'retry':'tv',m.fixture==='offline'?'rotate-cw':'gamepad-2',m.fixture==='offline'?'Retry connection':'TV controls').replace('</button>',`${m.fixture==='offline'?'Retry connection':'TV controls'}</button>`)}</div><div class="top">${button('exit','arrow-left','Return to room','exit')}<div class="room-label">Watch Kodi<small>Living room</small></div></div>`}
 if(m.exited)node.innerHTML=`<div class="empty-state exited">${icon('house')}<h3>Living room</h3><p>Playback continues on your TV.</p><button data-action="resume" class="room-card">${icon('play')}&nbsp; Return to Watch Kodi</button></div>`;
 node.insertAdjacentHTML('beforeend',sheet(m));
 // Dialog focus stays within the sheet; underlying transport cannot be clicked.
 for(const el of node.querySelectorAll(':scope > :not(.sheet):not(.sheet-scrim)'))el.inert=Boolean(m.sheet);
 if(m.sheet)node.querySelector('.sheet button:not(:disabled)')?.focus({preventScroll:true});
 else if(focusAction)node.querySelector(`[data-action="${focusAction}"]`)?.focus({preventScroll:true});
}
for(const [id,name,desc,tag] of concepts){
 document.getElementById('studies').insertAdjacentHTML('beforeend',`<article class="study" data-id="${id}"><div class="study-head"><h2>${id.toUpperCase()} / ${name}${tag?`<span class="tag">${tag}</span>`:''}</h2><p>${desc}</p></div><div class="viewport"><div id="remote-${id}" tabindex="-1"></div></div></article>`);render(id);
}
let active='a';
function notify(id,message){const node=document.getElementById('remote-'+id);node.querySelector('.notice')?.remove();const toast=document.createElement('div');toast.className='notice';toast.role='status';toast.textContent=message;node.append(toast);setTimeout(()=>toast.remove(),1200)}
function action(id,a){const m=models[id];if(a==='play')m.playing=!m.playing;
 else if(a==='rewind')m.position=Math.max(0,m.position-10);
 else if(a==='forward')m.position=Math.min(m.total,m.position+30);
 else if(['chapters','audio','subtitles','tv','more'].includes(a))m.sheet=a;
 else if(a==='close')m.sheet=null;
 else if(a==='exit'){if(m.sheet)m.sheet=null;else m.exited=true}
 else if(a==='resume')m.exited=false;
 else if(a.startsWith('chapter:')){m.position=chapters[Number(a.split(':')[1])][1];m.sheet=null}
 else if(a.startsWith('track:')){m[m.sheet]=a.slice(6);}
 else if(a.startsWith('kodi:')){notify(id,`Demo · ${a.slice(5)} sent to Kodi`);return}
 else if(a==='retry'){notify(id,'Demo · Kodi is still unavailable');return}
 render(id,a==='close'||a.startsWith('chapter:')?'play':a);
}
document.querySelectorAll('.remote').forEach(node=>{
 const id=node.id.slice(-1);
 node.addEventListener('pointerdown',()=>active=id);
 node.addEventListener('focusin',()=>active=id);
 node.addEventListener('click',e=>{const b=e.target.closest('[data-action]');if(b&&!b.disabled)action(id,b.dataset.action)});
 node.addEventListener('input',e=>{if(e.target.classList.contains('seek')){models[id].position=Number(e.target.value);updateProgress(id)}});
});
function updateProgress(id){const m=models[id],n=document.getElementById('remote-'+id);const s=n.querySelector('.seek');if(!s)return;s.value=m.position;s.style.setProperty('--played',`${m.position/m.total*100}%`);n.querySelector('[data-elapsed]').textContent=m.fixture==='live'?'LIVE':clock(m.position);n.querySelector('[data-remaining]').textContent=m.fixture==='live'?'Live broadcast':`−${clock(m.total-m.position)}`;}
document.getElementById('fixture').addEventListener('change',e=>{for(const [id,m] of Object.entries(models)){Object.assign(m,{fixture:e.target.value,playing:e.target.value!=='paused',sheet:null,exited:false,position:312});render(id)}});
document.getElementById('accent').addEventListener('input',e=>document.documentElement.style.setProperty('--accent',e.target.value));
document.querySelectorAll('[data-view]').forEach(b=>b.addEventListener('click',()=>{document.body.dataset.view=b.dataset.view;document.querySelectorAll('[data-view]').forEach(n=>n.classList.toggle('selected',n===b));if(b.dataset.view!=='all')active=b.dataset.view}));
document.addEventListener('keydown',e=>{
 if(e.target.closest('.workbench'))return;
 const m=models[active],node=document.getElementById('remote-'+active);
 if(e.key==='Escape'){e.preventDefault();action(active,m.sheet?'close':'exit');return}
 if(e.code==='Space'&&!['offline','idle'].includes(m.fixture)&&!m.exited){e.preventDefault();action(active,'play');return}
 if(m.sheet==='tv'&&(['ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Enter'].includes(e.key))){e.preventDefault();action(active,'kodi:'+(e.key==='Enter'?'Select':e.key.replace('Arrow','')));return}
 if(e.key==='Tab'&&m.sheet){const buttons=[...node.querySelectorAll('.sheet button:not(:disabled)')];if(!e.shiftKey&&document.activeElement===buttons.at(-1)){e.preventDefault();buttons[0].focus()}else if(e.shiftKey&&document.activeElement===buttons[0]){e.preventDefault();buttons.at(-1).focus()}return}
 if(!e.key.startsWith('Arrow'))return;
 if(e.target.classList.contains('seek')&&['ArrowLeft','ArrowRight'].includes(e.key))return;
 e.preventDefault();const items=[...node.querySelectorAll(m.sheet?'.sheet button:not(:disabled)': 'button:not(:disabled),input:not(:disabled)')].filter(n=>n.getClientRects().length&&!n.closest('[inert]'));
 const i=items.indexOf(document.activeElement),delta=['ArrowUp','ArrowLeft'].includes(e.key)?-1:1;const next=items[(i+delta+items.length)%items.length];next?.focus({preventScroll:true});if(m.sheet)next?.scrollIntoView({block:'nearest'});
});
setInterval(()=>{if(window.freezeSim)return;for(const [id,m]of Object.entries(models)){if(m.playing&&!['offline','idle','live'].includes(m.fixture)){m.position=Math.min(m.total,m.position+1);updateProgress(id)}}},1000);
