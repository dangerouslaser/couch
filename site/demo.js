/* The bezel drives the production Slint UI; all sample devices stay local. */
(() => {
  const frame = document.querySelector('#slint-demo');
  const screen = document.querySelector('#demo');
  const shell = document.querySelector('.device-shell');
  const sleep = document.querySelector('#demo-sleep');
  const loading = document.querySelector('#demo-loading');
  let ready = false, asleep = false, state = {}, interacted = false;
  let visible = false, tourTimer, tourIndex = 0;
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
  const scale = () => { frame.style.transform = `scale(${screen.clientWidth / 480})`; };
  new ResizeObserver(scale).observe(screen); scale();
  const send = name => {
    if (!ready) return;
    if (asleep) { asleep=false; sleep.hidden=true; return; }
    if (name==='power' && !state.player) { asleep=true; sleep.hidden=false; return; }
    frame.contentWindow.postMessage({couchPreview:'button',name},location.origin);
  };
  const stopTour = () => {
    interacted=true;clearTimeout(tourTimer);shell.dataset.autoplay='stopped';
    shell.querySelectorAll('.tour-pressed').forEach(b=>b.classList.remove('tour-pressed'));
  };
  // Use the same physical commands as a visitor. There is no second UI or video.
  const tour = [
    ['ok',1800], ['ok',1200], ['ok',1400], ['volume-up',1500],
    ['volume-up',1600], ['channel-up',2000], ['down',650], ['down',650],
    ['ok',2800], ['play',1800], ['play',2400], ['back-long',1600],
    ['back',1800], ['down',900], ['down',900], ['down',900], ['down',900],
    ['down',1400], ['ok',1800], ['back',1300], ['home',2200]
  ];
  const step = () => {
    if(interacted || reducedMotion.matches || !ready) return;
    if(!visible || document.hidden){tourTimer=setTimeout(step,500);return;}
    const [name,delay]=tour[tourIndex++ % tour.length];
    send(name);shell.dataset.autoplay='running';
    const button=shell.querySelector(`[data-remote="${name==='back-long'?'back':name}"]`);
    button?.classList.add('tour-pressed');
    setTimeout(()=>button?.classList.remove('tour-pressed'),name==='back-long'?650:180);
    tourTimer=setTimeout(step,delay);
  };
  const scheduleTour = () => {
    clearTimeout(tourTimer);
    if(ready && !interacted && !reducedMotion.matches) tourTimer=setTimeout(step,2200);
  };
  new IntersectionObserver(entries=>{visible=entries[0].isIntersecting;},{threshold:.25}).observe(shell);
  reducedMotion.addEventListener('change',()=>{if(reducedMotion.matches){clearTimeout(tourTimer);}else{scheduleTour();}});
  shell.addEventListener('pointerdown',stopTour,{capture:true});
  shell.addEventListener('keydown',stopTour,{capture:true});
  shell.addEventListener('wheel',stopTour,{passive:true});
  document.querySelectorAll('[data-remote]').forEach(button => {
    let pressed = 0, long = false;
    button.addEventListener('pointerdown',()=>{pressed=Date.now();long=false;});
    button.addEventListener('pointerup',()=>{long=button.dataset.remote==='back' && Date.now()-pressed>=600;});
    button.addEventListener('click',()=>{stopTour();send(long?'back-long':button.dataset.remote);});
  });
  sleep.addEventListener('click',()=>{asleep=false;sleep.hidden=true;});
  addEventListener('message',event=>{
    if(event.origin!==location.origin || event.source!==frame.contentWindow) return;
    if(event.data?.couchPreview==='ready') {ready=true;loading.hidden=true;scheduleTour();}
    if(event.data?.couchPreview==='interaction') stopTour();
    if(event.data?.couchPreview==='error') loading.textContent=event.data.message;
    if(event.data?.couchPreview==='state') {
      state=event.data.state;
      document.querySelector('#demo-summary').textContent=state.player?'Tears of Steel media controls':state.room!==null?`${['Living room','Kitchen','Bedroom','Office','Dining room','Hallway'][state.room]} devices. Ceiling lights ${state.level?`on at ${state.level}%`:'off'}.`:'Home room list.';
    }
  });
})();
