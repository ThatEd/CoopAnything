// F1 Manager 24 co-op bridge.
//
// Loaded by our pak override of Content/UIGameface/*.html. It has exactly one job: let an
// external process fire the same Gameface events the game's own buttons fire, so two clients can
// press Continue, advance the calendar and enter a race weekend together.
//
// Every button in this game is one of these calls:
//   MainMenu.js              onSelectContinue -> Engine.sendEvent('ContinueGame')
//   LoadSaveScreen.js:128    onSelectLoadData -> Engine.sendEvent('LoadGame', slotName)
//   TimeProgressionPanel.js  onAdvance        -> Engine.sendEvent('AdvanceToNextEvent')
//   CalendarContents.js:278                    -> Engine.sendEvent('NextMonthSelected')
// and Engine.sendEvent is `engine.trigger(name, ...args)`. A remote trigger of the same name is
// byte-for-byte the call the button makes: no input injection, no code patching, no hooking an
// address that moves every session, and no debugger that can wedge the process.
//
// Transport is a plain async GET to a loopback command server the operator runs. This is not a
// new capability for the UI: the shipped localisation loader uses XMLHttpRequest the same way.
//
// Facts established against a live game, each of which shaped this file:
//   * Synchronous XHR is blocked in Cohtml, so every request here is async.
//   * ContinueGame and LoadGame destroy the document that fired them, and Cohtml drops any
//     request still in flight during that teardown. So the report of "I ran it" is therefore
//     sent BEFORE the event, not after: waiting until after would always lose it.
//   * That means a synchronous ack is impossible for those two. The proof they ran is the page
//     change itself, which the next document reports on its first heartbeat.
//
// Failure policy: every network error is swallowed and retried with backoff. A missing command
// server must never affect the UI, so nothing here throws out of the poll loop.

(function () {
    'use strict';

    // Hot reload. Every change so far needed a game restart on both machines, which is the most
    // expensive thing in this project. The bridge re-fetches its own source when a revision file
    // changes and re-evaluates it, so behaviour can be changed by writing a file and waiting.
    //
    // A reload is a full re-run of this IIFE, so nothing may depend on state surviving it: `state`
    // is rebuilt and the poll loop restarts. The timer itself must survive, so it lives on window.
    var SELF = '/bridge.js';
    var REV_FILE = '/bridge.rev';
    var RELOAD_MS = 3000;

    function reloadSource() {
        var xr = new XMLHttpRequest();
        try {
            xr.open('GET', 'http://' + (HOST || CANDIDATES[0]) + ':' + PORT + SELF + '?t=' + Date.now(), true);
            xr.timeout = 5000;
            xr.onreadystatechange = function () {
                if (xr.readyState !== 4) return;
                if (xr.status !== 200 || !xr.responseText) return;
                try {
                    // Compiled before it runs: a syntax error in a live edit must not leave the
                    // page with no bridge at all.
                    (new Function(xr.responseText))();
                } catch (e) {
                    console.error('[coop] reload rejected: ' + e);
                }
            };
            xr.send();
        } catch (e) { /* not fatal */ }
    }
    if (!window.__coopReload) {
        window.__coopReload = { last: '' };
        setInterval(function () {
            var xr = new XMLHttpRequest();
            try {
                xr.open('GET', 'http://' + (window.__coopHost || '127.0.0.1') + ':' + (window.__coopPort || 45999) + REV_FILE + '?t=' + Date.now(), true);
                xr.timeout = 4000;
                xr.onreadystatechange = function () {
                    if (xr.readyState !== 4 || xr.status !== 200) return;
                    var rev = String(xr.responseText || '').trim();
                    if (rev && rev !== window.__coopReload.last) {
                        window.__coopReload.last = rev;
                        reloadSource();
                    }
                };
                xr.send();
            } catch (e) { /* ignore */ }
        }, RELOAD_MS);
    }

    var HOST = null;                 // resolved on the first successful poll
    var PORT = 45999;

    var POLL_IDLE = 250;      // ms between polls when nothing is happening
    var POLL_BUSY = 120;      // ms between polls right after a command

    // Which machine runs the command server. Both clients use the same pak, so it cannot be a single
    // committed address: on the host, loopback is right, and on the other machine loopback has nothing
    // listening. The bridge probes in order and keeps the first that answers, so one pak works on
    // either side.
    //
    // The host's address is per-install, so it is read from `localStorage` under `coop.host` rather than
    // committed here - a LAN IP in the repo is wrong for everyone else and points at a stranger's
    // machine. Setting it once on the second machine is enough (see mod/README.md).
    var CANDIDATES = (function () {
        var list = ['127.0.0.1'];        // same machine as the server (the host)
        try {
            var saved = String(window.localStorage.getItem('coop.host') || '').trim();
            saved = saved.replace(/^https?:\/\//, '').replace(/:\d+$/, '');
            if (saved && saved !== '127.0.0.1') list.push(saved);
        } catch (e) { /* no storage: loopback only */ }
        return list;
    })();

    // Which document this is. Frontend.html is the menu UI; RaceWeekend.html and friends are the
    // in-game UIs. The page name tells the lobby which screen a client is on without reading game
    // memory, and a change of page name is how the lobby confirms a destructive command landed.
    var PAGE = (function () {
        try {
            var p = String(window.location.pathname || '');
            var m = p.match(/([A-Za-z]+)\.html/);
            if (m) return m[1];
            return p || 'unknown';
        } catch (e) { return 'unknown'; }
    })();

    var state = {
        stage: 'loading',
        sent: 0,
        failures: 0,
        backoff: 1,
        polling: false,
        seenEngine: false,
        lastHadWork: false
    };

    function url(path, query) {
        return 'http://' + (HOST || CANDIDATES[0]) + ':' + PORT + path + (query ? '?' + query : '');
    }

    function urlFor(host, path, query) {
        return 'http://' + host + ':' + PORT + path + (query ? '?' + query : '');
    }

    function enc(s) {
        return encodeURIComponent(String(s));
    }

    function engineReady() {
        return typeof engine !== 'undefined' && engine && typeof engine.trigger === 'function';
    }

    function send(path, query) {
        var xr = new XMLHttpRequest();
        try {
            xr.open('GET', url(path, query), true);
            xr.timeout = 5000;
            xr.send();
        } catch (e) { /* the UI must not care whether the operator is running */ }
    }

    // Poll: identify this document, and take the next command if there is one.
    function poll() {
        var q = 'client=' + enc(HOST || '?') +
            '&page=' + enc(PAGE) +
            '&stage=' + enc(state.stage) +
            '&sent=' + state.sent +
            '&pkg=coop-bridge' +
            '&v=6';

        // No host resolved yet: probe the candidates in order and keep the first that answers.
        if (!HOST) {
            HOST = window.__coopHost || null;
        }
        if (!HOST) {
            probeHosts(0);
            return;
        }

        var xr = new XMLHttpRequest();
        var done = false;

        function schedule() {
            if (done) return;
            done = true;
            setTimeout(poll, state.lastHadWork ? POLL_BUSY : POLL_IDLE);
        }

        try {
            xr.open('GET', url('/hello', q), true);
            xr.timeout = 5000;
            xr.onreadystatechange = function () {
                if (xr.readyState !== 4) return;
                state.lastHadWork = false;
                if (xr.status === 200) {
                    state.failures = 0;
                    state.backoff = 1;
                    if (xr.responseText) {
                        state.lastHadWork = true;
                        // Run outside the XHR callback: dispatching inside it would let a command
                        // that tears the document down re-enter the destroyed view's handlers.
                        var next = xr.responseText;
                        setTimeout(function () { dispatch(next); }, 0);
                    }
                    schedule();
                } else {
                    // Not fatal and not unusual: the view is between documents.
                    state.failures++;
                    state.backoff = Math.min(state.backoff + 1, 20);
                    setTimeout(poll, 400 * state.backoff);
                }
            };
            xr.send();
        } catch (e) {
            state.failures++;
            setTimeout(poll, 500);
        }
    }

    function probeHosts(i) {
        if (i >= CANDIDATES.length) {
            HOST = null;
            setTimeout(poll, 2000);
            return;
        }
        var candidate = CANDIDATES[i];
        var xr = new XMLHttpRequest();
        var settled = false;
        function next() {
            if (settled) return;
            settled = true;
            probeHosts(i + 1);
        }
        try {
            xr.open('GET', urlFor(candidate, '/hello',
                'client=' + enc('probe') + '&page=' + enc(PAGE) + '&stage=probing' +
                '&pkg=coop-bridge&v=6'), true);
            xr.timeout = 3000;
            xr.onreadystatechange = function () {
                if (xr.readyState !== 4) return;
                if (xr.status === 200) {
                    settled = true;
                    HOST = candidate;
                    window.__coopHost = candidate;
                    state.stage = 'engine-ready';
                    report('probe', true, 'attached to ' + candidate);
                    setTimeout(poll, 0);
                } else {
                    next();
                }
            };
            xr.send();
        } catch (e) {
            next();
        }
    }

    // Which commands are legal on which document.
    //
    // This is not a guess: firing LoadGame from the in-game views (RaceWeekend.html, TeamManagement.html)
    // tore the front-end down and never rebuilt it, leaving a black screen with the process still
    // alive. The game's own load path only ever runs from Frontend.html - LoadSaveScreen lives in the
    // front-end app - so that is the only document allowed to issue it.
    //
    // ContinueGame is in the same category: MainMenu.js is a front-end component.
    var LOAD_FROM = ['Frontend'];
    var MENU_FROM = ['Frontend'];

    function allowedFor(cmd, page) {
        if (cmd === 'load' || cmd === 'loadgame') return LOAD_FROM.indexOf(page) >= 0;
        if (cmd === 'continue' || cmd === 'continuegame') return MENU_FROM.indexOf(page) >= 0;
        return true;
    }

    // Command grammar is tiny and whitespace-split, so a shell, a .bat file or the Rust lobby can
    // all produce it. Event names pass through unchanged: this is a thin remote for
    // engine.trigger, deliberately not a reimplementation of the game's commands.
    //
    //   Continue               ContinueGame               main-menu Continue      (Frontend only)
    //   Load [slot]            LoadGame <slot>            autosave, save1..29     (Frontend only)
    //   Advance                AdvanceToNextEvent         the in-career Advance button
    //   NextMonth              NextMonthSelected          the calendar month button
    //   Raw <event> [arg...]   any Gameface event by name
    //   Stage <name>           report-only
    function dispatch(text) {
        var line = String(text).replace(/[\r\n]+/g, ' ').trim();
        if (!line) return;
        var parts = line.split(/\s+/);
        var cmd = parts[0].toLowerCase();

        if (!engineReady()) {
            report(line, false, 'engine.trigger unavailable on ' + PAGE);
            return;
        }

        if (cmd === 'stage') {
            state.stage = parts[1] || state.stage;
            report(line, true, 'stage ' + state.stage);
            return;
        }

        // Reads the live DOM and reports it. This is how the *actual* pre-session screen is
        // identified instead of guessed: the game's session start is not a Gameface event, so the
        // next step has to be chosen from what is really on screen.
        if (cmd === 'dom') {
            var out = [];
            try {
                var all = document.querySelectorAll('*');
                for (var i = 0; i < all.length && out.length < 60; i++) {
                    var el = all[i];
                    var cls = el.className;
                    if (typeof cls !== 'string' || !cls) continue;
                    if (/root$|_root|Panel|Menu|Header|Button|Layer/.test(cls)) {
                        var txt = (el.textContent || '').replace(/\s+/g, ' ').trim();
                        out.push(cls + ' | ' + txt.substring(0, 70));
                    }
                }
            } catch (e) {
                out.push('ERR ' + e);
            }
            report(line, true, 'PAGE ' + PAGE + ' :: ' + out.join(' ;; '));
            return;
        }

        if (!allowedFor(cmd, PAGE)) {
            // Refuse rather than try: this exact call is what black-screened a live client.
            report(line, false, 'refused on ' + PAGE + ' (needs ' +
                (cmd === 'advance' ? 'any page' : 'Frontend') + ')');
            return;
        }

        var eventName = null;
        var eventArgs = [];

        if (cmd === 'continue' || cmd === 'continuegame') {
            eventName = 'ContinueGame';
        } else if (cmd === 'load' || cmd === 'loadgame') {
            eventName = 'LoadGame';
            eventArgs = [parts[1] || 'autoSave'];
        } else if (cmd === 'advance') {
            // The Advance button is a state machine, not one event: ContinueButtonContainer decides
            // per press (needs-a-response -> inbox, sponsor targets -> its own screen, race weekend
            // -> AdvanceToNextEvent). On the race-weekend screens the press that actually starts the
            // session is AdvanceToNextEvent, sometimes preceded by other UI steps, so a single
            // Advance from the bridge can be swallowed with no error. `advancen` sends it N times,
            // spaced, which is what "advance until the sim steps" needs in practice.
            var times = 1;
            if (parts[1]) {
                var parsed = parseInt(parts[1], 10);
                if (parsed > 0 && parsed <= 20) times = parsed;
            }
            for (var i = 0; i < times; i++) {
                (function (n) {
                    setTimeout(function () {
                        try { engine.trigger('AdvanceToNextEvent'); } catch (e) { /* view gone */ }
                    }, n * 1200);
                })(i);
            }
            eventName = 'AdvanceToNextEvent';
            state.sent += times - 1;
        } else if (cmd === 'nextmonth') {
            eventName = 'NextMonthSelected';
        } else if (cmd === 'raw') {
            if (!parts[1]) {
                report(line, false, 'Raw needs an event name');
                return;
            }
            eventName = parts[1];
            eventArgs = parts.slice(2);
        } else if (cmd === 'menu' || cmd === 'mainmenu') {
            eventName = 'QuitGame';
        } else {
            report(line, false, 'unknown command');
            return;
        }

        // Report BEFORE firing. ContinueGame and LoadGame replace this document, and Cohtml drops
        // in-flight requests during that teardown; a report sent afterwards never arrives.
        state.sent++;
        report(line, true, PAGE + ' -> ' + eventName +
            (eventArgs.length ? ' ' + eventArgs.join(' ') : ''));

        try {
            engine.trigger.apply(engine, [eventName].concat(eventArgs));
        } catch (e) {
            // On the success path this document is already going away, so there is no way to
            // report a throw. The operator sees the absence of a page change instead.
            return;
        }
    }

    function report(line, ok, detail) {
        var payload = (ok ? 'OK ' : 'ERR ') + line + ' :: ' + detail;
        send('/report', 'page=' + enc(PAGE) + '&text=' + enc(payload));
    }

    // Screen tracking by DOM. Each UI page renders its own root; the front-end's main menu is the
    // only screen with MainMenu_root.
    function watchScreens() {
        try {
            if (document.querySelector('.MainMenu_root')) {
                if (state.stage !== 'mainmenu') state.stage = 'mainmenu';
            } else if (PAGE !== 'Frontend') {
                if (state.stage !== 'ingame') state.stage = 'ingame';
            } else if (state.stage === 'loading' || state.stage === 'engine-ready') {
                state.stage = 'frontend';
            }
        } catch (e) { /* DOM not ready */ }
        setTimeout(watchScreens, 500);
    }

    function start() {
        if (!engineReady()) {
            setTimeout(start, 250);
            return;
        }
        // Publish the resolved host/port on window so the reload timer keeps talking to the right
        // machine after a re-evaluation.
        window.__coopPort = PORT;

        if (!state.seenEngine) {
            state.seenEngine = true;
            state.stage = 'engine-ready';
            send('/pak-ok', 'file=' + enc(PAGE) + '.html&bridge=coop');
        }
        watchScreens();

        // One poll loop per document. A reload re-evaluates this whole IIFE, so starting a second
        // loop would race the first over the same timer and could leave neither running - which is
        // exactly what happened: the host went silent for eight minutes after a live edit.
        if (!window.__coopPolling) {
            window.__coopPolling = true;
            setTimeout(poll, 0);
        }
    }

    start();
})();
