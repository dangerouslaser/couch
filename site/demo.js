/* Local sample data only: this demo never contacts a device or saves a setting. */
(() => {
    const screen = document.querySelector('#demo');
    const viewport = document.querySelector('#demo-viewport');
    const title = document.querySelector('#demo-title');
    const toast = document.querySelector('#demo-toast');
    const sleep = document.querySelector('#demo-sleep');
    const controls = document.querySelector('.demo-controls');
    const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
    const icon = name => `<span class="icon icon-${name}" aria-hidden="true"></span>`;
    const light = (name, on, brightness) => ({ name, type: 'light', on, brightness });
    const rooms = [
        { name: 'Living room', icon: 'house', devices: [light('Floor lamp', true, 65), light('Table lamp', false, 40), {name:'Kodi', type:'media', on:true}, {name:'LG TV', type:'tv', on:true}] },
        { name: 'Kitchen', icon: 'sun', devices: [light('Pendant lights', true, 80), light('Counter lights', false, 50)] },
        { name: 'Bedroom', icon: 'moon', devices: [light('Bedside lamp', false, 35), light('Ceiling light', false, 70), {name:'Bedroom TV', type:'tv', on:false}] },
        { name: 'Office', icon: 'briefcase-business', devices: [light('Desk lamp', false, 75), light('Shelf lights', false, 45)] },
        { name: 'Dining room', icon: 'sofa', devices: [light('Dining lights', true, 60), light('Sideboard', false, 30)] },
        { name: 'Hallway', icon: 'lightbulb', devices: [light('Hall lights', false, 80)] }
    ];
    const scenes = [ {name:'Relax', brightness:35}, {name:'Bright', brightness:100}, {name:'Lights off', brightness:0} ];
    let route = {page:'home'};
    const history = [];
    const positions = new Map();
    let awake = true;
    let selectedDevice = 0;
    let playing = true;
    let elapsed = 754;
    let toastTimer;
    let transition;
    const listSize = new ResizeObserver(entries => {
        for (const {target} of entries) {
            const available = target.clientHeight - 8;
            const rows = Math.max(1, Math.floor((available + 9) / 75));
            target.style.setProperty('--row-size', `${(available - (rows - 1) * 9) / rows}px`);
        }
    });
    const routeKey = value => `${value.page}:${value.room ?? ''}`;
    const time = value => `${Math.floor(value / 60)}:${String(value % 60).padStart(2, '0')}`;

    function notify(message) {
        clearTimeout(toastTimer);
        toast.textContent = message;
        toast.classList.add('shown');
        toastTimer = setTimeout(clearToast, 1800);
    }
    function clearToast() {
        clearTimeout(toastTimer);
        toast.classList.remove('shown');
        toast.textContent = '';
    }
    function savePosition() {
        positions.set(routeKey(route), viewport.querySelector('.demo-list')?.scrollTop || 0);
    }
    function navigate(next, back = false) {
        savePosition();
        if (!back) history.push({...route});
        route = next;
        selectedDevice = 0;
        clearToast();
        render(back ? -1 : 1, true);
    }
    function back() {
        if (history.length) navigate(history.pop(), true);
        else notify('You’re on the home screen');
    }
    function sceneButton() {
        return `<div class="scenes-label">SCENES</div><button class="scene-row" data-action="scenes">${icon('sparkles')}<strong>3 scenes</strong>${icon('chevron-right')}</button>`;
    }
    function roomRows() {
        return rooms.map((room, index) => {
            const count = room.devices.filter(device => device.on).length;
            return `<button class="room ${count ? '' : 'idle'}" data-action="room" data-index="${index}"><span class="room-symbol ${count ? 'active' : ''}">${icon(room.icon)}</span><span class="row-label">${room.name}<small>${room.devices.length} devices</small></span><span class="room-state">${count ? icon('circle-dot') + count + ' ON' : 'IDLE'}</span></button>`;
        }).join('');
    }
    function deviceRows() {
        return rooms[route.room].devices.map((device, index) => {
            const glyph = device.type === 'light' ? 'lightbulb' : device.type === 'media' ? 'play' : 'tv';
            const state = device.type === 'media' ? 'Open controls' : device.on ? `On${device.type === 'light' ? ' · ' + device.brightness + '%' : ''}` : 'Off';
            return `<button class="room ${device.on ? '' : 'idle'} ${index === selectedDevice ? 'selected' : ''}" data-action="device" data-index="${index}" ${device.type !== 'media' ? `aria-pressed="${device.on}"` : ''}><span class="room-symbol ${device.on ? 'active' : ''}">${icon(glyph)}</span><span class="row-label">${device.name}<small>${state}</small></span>${device.type === 'media' ? icon('chevron-right') : icon('power')}</button>`;
        }).join('');
    }
    function brightnessControl() {
        const device = rooms[route.room].devices[selectedDevice];
        if (device.type !== 'light') return '<p class="demo-hint">Tap a device to control it.</p>';
        return `<label class="brightness-control">${icon('sun')}<span>Brightness</span><output id="brightness-value">${device.brightness}%</output><input aria-label="${device.name} brightness" type="range" min="1" max="100" value="${device.brightness}" data-action="brightness"></label>`;
    }
    function mediaPage() {
        return `<div class="media-art">${icon('tv')}<span>EXAMPLE PLAYBACK</span><h2>Movie night</h2><p>Kodi · Living room</p></div><div class="media-deck"><label class="demo-hint" for="playback-progress">Playback position</label><input id="playback-progress" aria-label="Playback position" type="range" min="0" max="7200" value="${elapsed}" data-action="seek"><div class="media-time"><span id="elapsed">${time(elapsed)}</span><span>120:00</span></div><div class="media-buttons"><button data-action="skip" data-seconds="-30" aria-label="Skip back 30 seconds">${icon('skip-back')}</button><button data-action="play" aria-label="${playing ? 'Pause' : 'Play'}">${icon(playing ? 'pause' : 'play')}</button><button data-action="skip" data-seconds="30" aria-label="Skip forward 30 seconds">${icon('skip-forward')}</button></div><p class="demo-hint">Try playback, then Back to return.</p></div>`;
    }
    function render(direction = 0, focus = false) {
        transition?.cancel();
        title.textContent = route.page === 'home' ? 'HOME' : route.page === 'room' ? rooms[route.room].name : route.page === 'scenes' ? 'SCENES' : 'WATCH TV';
        const page = document.createElement('div');
        page.className = 'demo-page home-content';
        if (route.page === 'home') {
            page.innerHTML = `<button class="activity-row" data-action="media">${icon('play')}<span class="row-label">Watch TV<small>KODI · LIVING ROOM</small></span>${icon('chevron-right')}</button><div class="demo-list" role="group" aria-label="Rooms">${roomRows()}</div><p class="demo-hint">Scroll for more rooms</p>${sceneButton()}`;
        } else if (route.page === 'room') {
            page.innerHTML = `<div class="demo-list" role="group" aria-label="Devices">${deviceRows()}</div>${brightnessControl()}${sceneButton()}`;
        } else if (route.page === 'scenes') {
            page.innerHTML = `<p class="demo-hint">${route.room === undefined ? 'All rooms' : rooms[route.room].name}</p><div class="demo-list">${scenes.map((scene, index) => `<button class="room" data-action="scene" data-index="${index}"><span class="room-symbol active">${icon('sparkles')}</span><span class="row-label">${scene.name}<small>${scene.brightness ? scene.brightness + '% brightness' : 'Turn off the lights'}</small></span>${icon('chevron-right')}</button>`).join('')}</div><p class="demo-hint">Scenes change the sample lights.<br>Back returns to the previous screen.</p>`;
        } else {
            page.classList.add('media-page');
            page.innerHTML = mediaPage();
        }
        listSize.disconnect();
        viewport.replaceChildren(page);
        const list = page.querySelector('.demo-list');
        if (list) {
            listSize.observe(list);
            requestAnimationFrame(() => { list.scrollTop = positions.get(routeKey(route)) || 0; });
        }
        if (direction && !reduceMotion.matches) {
            transition = page.animate([{transform:`translateX(${direction * 100}%)`}, {transform:'translateX(0)'}], {duration:220, easing:'cubic-bezier(.2,.7,.2,1)'});
        }
        if (focus) page.querySelector('button')?.focus({preventScroll:true});
    }
    function wake() {
        awake = true;
        sleep.hidden = true;
        viewport.inert = false;
        controls.querySelector('[data-remote="power"]').setAttribute('aria-label', 'Put demo screen to sleep');
    }
    viewport.addEventListener('click', event => {
        const button = event.target.closest('button[data-action]');
        if (!button || !awake) return;
        const index = Number(button.dataset.index);
        switch (button.dataset.action) {
            case 'room': navigate({page:'room',room:index}); break;
            case 'media': navigate({page:'media'}); break;
            case 'scenes': navigate({page:'scenes',room:route.room}); break;
            case 'device': {
                const device = rooms[route.room].devices[index];
                if (device.type === 'media') { navigate({page:'media'}); break; }
                selectedDevice = index;
                device.on = !device.on;
                savePosition();
                render();
                viewport.querySelector(`[data-action="device"][data-index="${index}"]`).focus({preventScroll:true});
                break;
            }
            case 'scene': {
                const scene = scenes[index];
                for (const room of route.room === undefined ? rooms : [rooms[route.room]]) {
                    for (const device of room.devices.filter(device => device.type === 'light')) {
                        device.on = scene.brightness > 0;
                        if (device.on) device.brightness = scene.brightness;
                    }
                }
                back();
                notify(`${scene.name} applied`);
                break;
            }
            case 'play': playing = !playing; render(); viewport.querySelector('[data-action="play"]').focus({preventScroll:true}); break;
            case 'skip': elapsed = Math.max(0,Math.min(7200,elapsed + Number(button.dataset.seconds))); updatePlayback(); break;
        }
    });
    viewport.addEventListener('input', event => {
        const input = event.target;
        if (input.dataset.action === 'brightness') {
            const device = rooms[route.room].devices[selectedDevice];
            device.brightness = Number(input.value);
            device.on = true;
            document.querySelector('#brightness-value').textContent = `${device.brightness}%`;
            const row = viewport.querySelector(`[data-action="device"][data-index="${selectedDevice}"]`);
            row.classList.remove('idle');
            row.setAttribute('aria-pressed','true');
            row.querySelector('.room-symbol').classList.add('active');
            row.querySelector('small').textContent = `On · ${device.brightness}%`;
        } else if (input.dataset.action === 'seek') {
            elapsed = Number(input.value);
            updatePlayback();
        }
    });
    viewport.addEventListener('keydown', event => {
        if (event.key === 'Escape') { event.preventDefault(); back(); }
        if (!['ArrowUp','ArrowDown'].includes(event.key) || event.target.matches('input')) return;
        const buttons = [...viewport.querySelectorAll('button')];
        const index = buttons.indexOf(document.activeElement);
        if (index < 0) return;
        event.preventDefault();
        const next = buttons[Math.max(0, Math.min(buttons.length - 1,index + (event.key === 'ArrowDown' ? 1 : -1)))];
        next.focus({preventScroll:true});
        // Scroll only the remote's list; leave the surrounding website still.
        const list = next.closest('.demo-list');
        if (list) {
            const top = next.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop;
            const bottom = top + next.offsetHeight;
            if (top < list.scrollTop || bottom > list.scrollTop + list.clientHeight) list.scrollTo({top:top < list.scrollTop ? top : bottom - list.clientHeight,behavior:reduceMotion.matches ? 'instant' : 'smooth'});
        }
    });
    controls.addEventListener('click', event => {
        const button = event.target.closest('[data-remote]');
        if (!button) return;
        if (!awake) { wake(); return; }
        if (button.dataset.remote === 'power') {
            clearToast(); awake = false; sleep.hidden = false; viewport.inert = true;
            button.setAttribute('aria-label','Wake demo screen');
        } else if (button.dataset.remote === 'home') {
            history.length = 0; navigate({page:'home'}, true);
        } else back();
    });
    sleep.addEventListener('click', () => { wake(); controls.querySelector('[data-remote="power"]').focus({preventScroll:true}); });
    function updatePlayback() {
        if (route.page !== 'media') return;
        const progress = document.querySelector('#playback-progress');
        progress.value = elapsed;
        document.querySelector('#elapsed').textContent = time(elapsed);
    }
    setInterval(() => {
        if (awake && playing && route.page === 'media' && !document.hidden && elapsed < 7200) { elapsed++; updatePlayback(); }
    }, 1000);
    render();
})();
