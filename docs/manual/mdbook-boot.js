// The inline scripts of mdBook's default page template, moved into one same-origin file so the
// Content-Security-Policy (`script-src 'self'`, set in theme/index.hbs) needs no 'unsafe-inline'.
// Repository tooling, not generated, and not vendored: it mirrors the pinned mdBook's template, so
// refresh it together with theme/index.hbs when `mdbook_version` in the justfile moves.
//
// theme/index.hbs loads this file four times, each tag naming the template block it replaces in
// `data-phase`. The values those blocks interpolated arrive as data-* attributes on <html>. The
// top-level names (path_to_root, html, sidebar, ...) stay globals because mdBook's book.js and
// toc.js read them.
(() => {
    const root = document.documentElement;
    const phase = document.currentScript && document.currentScript.dataset.phase;

    if (phase === 'head') {
        // "Provide site root and default themes to javascript".
        window.path_to_root = root.dataset.pathToRoot;
        window.default_light_theme = root.dataset.defaultLightTheme;
        window.default_dark_theme = root.dataset.defaultDarkTheme;
        if (root.dataset.searchindex !== undefined) {
            window.path_to_searchindex_js = root.dataset.searchindex;
        }

        // "Work around some values being stored in localStorage wrapped in quotes".
        try {
            for (const key of ['mdbook-theme', 'mdbook-sidebar']) {
                const value = localStorage.getItem(key);
                if (value.startsWith('"') && value.endsWith('"')) {
                    localStorage.setItem(key, value.slice(1, value.length - 1));
                }
            }
        } catch (e) { }

        // "Set the theme before any content is loaded, prevents flash".
        const preferred = window.matchMedia('(prefers-color-scheme: dark)').matches
            ? window.default_dark_theme : window.default_light_theme;
        window.default_theme = preferred; // book.js reads this global
        let theme;
        try { theme = localStorage.getItem('mdbook-theme'); } catch (e) { }
        if (theme === null || theme === undefined) { theme = preferred; }
        root.classList.remove(window.default_light_theme);
        root.classList.add(theme);
        root.classList.add('js');
    } else if (phase === 'sidebar') {
        // "Hide / unhide sidebar before it is displayed".
        let sidebar = null;
        const toggle = document.getElementById('mdbook-sidebar-toggle-anchor');
        if (document.body.clientWidth >= 1080) {
            try { sidebar = localStorage.getItem('mdbook-sidebar'); } catch (e) { }
            sidebar = sidebar || 'visible';
        } else {
            sidebar = 'hidden';
            toggle.checked = false;
        }
        if (sidebar === 'visible') {
            toggle.checked = true;
        } else {
            root.classList.remove('sidebar-visible');
        }
        window.sidebar = sidebar;
        window.sidebar_toggle = toggle;
    } else if (phase === 'aria') {
        // "Apply ARIA attributes after the sidebar and the sidebar toggle button are added".
        document.getElementById('mdbook-sidebar-toggle')
            .setAttribute('aria-expanded', window.sidebar === 'visible');
        document.getElementById('mdbook-sidebar')
            .setAttribute('aria-hidden', window.sidebar !== 'visible');
        Array.from(document.querySelectorAll('#mdbook-sidebar a')).forEach((link) => {
            link.setAttribute('tabIndex', window.sidebar === 'visible' ? 0 : -1);
        });
    } else if (phase === 'flags') {
        // Livereload (only `mdbook serve` sets it), the playground flags, print and redirects.
        const endpoint = root.dataset.liveReload;
        if (endpoint) {
            const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
            const socket = new WebSocket(protocol + '//' + location.host + '/' + endpoint);
            socket.onmessage = (event) => {
                if (event.data === 'reload') {
                    socket.close();
                    location.reload();
                }
            };
            window.onbeforeunload = () => socket.close();
        }
        if (root.dataset.playgroundLineNumbers !== undefined) {
            window.playground_line_numbers = true;
        }
        if (root.dataset.playgroundCopyable !== undefined) {
            window.playground_copyable = true;
        }
        if (root.dataset.print !== undefined) {
            window.addEventListener('load', () => {
                if (window.MathJax && window.MathJax.Hub) {
                    window.MathJax.Hub.Register.StartupHook('End', () => {
                        window.setTimeout(window.print, 100);
                    });
                } else {
                    window.setTimeout(window.print, 100);
                }
            });
        }
        if (root.dataset.fragmentMap !== undefined) {
            document.addEventListener('DOMContentLoaded', () => {
                const target = JSON.parse(root.dataset.fragmentMap)[window.location.hash];
                if (target) {
                    window.location.replace(new URL(target, window.location.href).href);
                }
            });
        }
    }
})();
