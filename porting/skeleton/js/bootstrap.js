// bootstrap.js — minimal browser shim for the obscura-js skeleton.
// Replace with the real 8001-line bootstrap.js from the upstream obscura repo.
// This minimal version exercises the bridge: console, crypto, window/document aliases.

(function() {
    // --- window / globalThis alias ---
    var global = globalThis;
    global.window = global;
    global.self = global;

    // --- console ---
    var console = {
        log:   function() { var a = Array.prototype.join.call(arguments, ' '); Deno.core.ops.op_console_msg('info', a); },
        info:  function() { var a = Array.prototype.join.call(arguments, ' '); Deno.core.ops.op_console_msg('info', a); },
        warn:  function() { var a = Array.prototype.join.call(arguments, ' '); Deno.core.ops.op_console_msg('warn', a); },
        error: function() { var a = Array.prototype.join.call(arguments, ' '); Deno.core.ops.op_console_msg('error', a); },
        debug: function() { var a = Array.prototype.join.call(arguments, ' '); Deno.core.ops.op_console_msg('info', a); },
    };
    global.console = console;

    // --- crypto ---
    var crypto = {
        getRandomValues: function(arr) {
            var bytes = Deno.core.ops.op_random_bytes(arr.length);
            for (var i = 0; i < arr.length; i++) arr[i] = bytes[i];
            return arr;
        },
        subtle: {
            digest: function(alg, data) {
                return Promise.resolve(Deno.core.ops.op_subtle_digest(alg, data));
            },
        },
    };
    global.crypto = crypto;

    // --- document stub ---
    var document = {
        title: '',
        URL: 'about:blank',
        cookie: '',
        readyState: 'complete',
        createElement: function(tag) { return { tagName: tag.toUpperCase(), children: [], setAttribute: function(){}, appendChild: function(){} }; },
        getElementById: function() { return null; },
        querySelector: function() { return null; },
        querySelectorAll: function() { return []; },
        addEventListener: function() {},
    };
    global.document = document;

    // --- navigator stub ---
    global.navigator = {
        userAgent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36',
        platform: 'Linux aarch64',
    };

    // --- location stub ---
    global.location = { href: 'about:blank', origin: 'null', protocol: 'about:', host: '', hostname: '', port: '', pathname: '/about/blank', search: '', hash: '' };

    // --- setTimeout / setInterval (basic) ---
    var timerId = 0;
    var timers = {};
    global.setTimeout = function(fn, ms) {
        var id = ++timerId;
        // QuickJS job queue — fires on next microtask tick
        Promise.resolve().then(function() { fn(); });
        return id;
    };
    global.clearTimeout = function(id) { delete timers[id]; };
    global.setInterval = function(fn, ms) {
        var id = ++timerId;
        return id;
    };
    global.clearInterval = function(id) {};

})();
