// Makes every rendered Mermaid diagram zoomable and pannable with the vendored svg-pan-zoom
// (svg-pan-zoom.min.js, provenance in book.toml). Repository tooling, not generated: it is not
// part of `mdbook-mermaid install`, so a refresh of mermaid-init.js leaves it alone.
//
// A diagram keeps its normal size, and the page keeps scrolling normally, until the reader asks:
//   - the +, - and reset controls in the diagram's corner zoom it;
//   - Ctrl (or Cmd) + mouse wheel, or a trackpad pinch, zooms at the pointer;
//   - once zoomed in, dragging pans; at normal size a drag does nothing, so touch scrolling works.
//
// svg-pan-zoom BSD 2-Clause licence: Copyright 2009-2010 Andrea Leofreddi (full text in
// svg-pan-zoom.LICENSE, shipped with the source tree).
//
// Mermaid renders asynchronously and can insert, move and replace a block's <svg> several times
// before it settles, while svg-pan-zoom measures the <svg> once, when it attaches (and leaves a
// broken transform behind if it measured a half-rendered one). So each block is attached only
// after its content has stopped changing for SETTLE_MS, and a failed attempt is undone and retried.
(() => {
    if (typeof svgPanZoom !== 'function') {
        return;
    }

    const SETTLE_MS = 250;
    const RETRY_MS = 250;
    const MAX_RETRIES = 20;

    const options = {
        controlIconsEnabled: true,
        zoomEnabled: true,
        panEnabled: false,
        mouseWheelZoomEnabled: false,
        dblClickZoomEnabled: false,
        preventMouseEventsDefault: false,
        fit: true,
        center: true,
        minZoom: 1,
        maxZoom: 12,
        zoomScaleSensitivity: 0.3,
    };

    // Put the <svg> back as Mermaid made it after a failed attach.
    const undo = (svg) => {
        const viewport = svg.querySelector(':scope > g.svg-pan-zoom_viewport');
        if (viewport) {
            while (viewport.firstChild) {
                svg.insertBefore(viewport.firstChild, viewport);
            }
            viewport.remove();
        }
        svg.querySelectorAll(':scope > #svg-pan-zoom-controls').forEach((node) => node.remove());
    };

    const enhance = (svg, zoom) => {
        // Pan only while zoomed in, so a drag or a touch-scroll over a diagram at normal size
        // still belongs to the page.
        const normal = zoom.getZoom();
        const home = zoom.getPan();
        zoom.setOnZoom((level) => {
            const zoomedIn = level > normal * 1.001;
            if (zoomedIn) {
                zoom.enablePan();
            } else {
                zoom.disablePan();
            }
            if (svg.classList.contains('pan-zoom-zoomed') !== zoomedIn) {
                // A wide, short diagram is a thin strip: give a zoomed one room (see the CSS),
                // and tell svg-pan-zoom its frame changed. Outside the callback, which must not
                // call the zoom API.
                svg.classList.toggle('pan-zoom-zoomed', zoomedIn);
                setTimeout(() => {
                    zoom.resize();
                    if (zoomedIn) {
                        zoom.center();
                    } else {
                        zoom.zoom(normal);
                        zoom.pan(home);
                    }
                }, 0);
            }
        });

        svg.addEventListener('wheel', (event) => {
            if (!(event.ctrlKey || event.metaKey)) {
                return; // a plain wheel scrolls the page
            }
            event.preventDefault();
            const box = svg.getBoundingClientRect();
            const factor = Math.exp(-event.deltaY * (event.deltaMode === 1 ? 0.05 : 0.0025));
            zoom.zoomAtPointBy(factor, {
                x: event.clientX - box.left,
                y: event.clientY - box.top,
            });
        }, { passive: false });
    };

    const attach = (block, retriesLeft = MAX_RETRIES) => {
        const svg = block.querySelector('svg');
        if (!svg || svg.dataset.panZoom === 'ready' || svg.dataset.panZoom === 'failed') {
            return;
        }
        try {
            const box = svg.getBoundingClientRect();
            if (box.width === 0 || box.height === 0) {
                throw new Error('the diagram is not laid out yet');
            }
            // Mermaid sizes the <svg> only by width="100%" and a max-width; its height comes from
            // the viewBox's aspect ratio. svg-pan-zoom removes the viewBox, which collapses the
            // <svg> to the browser's default 150px height: a taller diagram is clipped, and the
            // controls, placed from the height measured before, land in the clipped part. Pin the
            // ratio first so the <svg> keeps its height.
            const viewBox = svg.viewBox && svg.viewBox.baseVal;
            if (viewBox && viewBox.width > 0 && viewBox.height > 0) {
                svg.style.aspectRatio = `${viewBox.width} / ${viewBox.height}`;
            }
            enhance(svg, svgPanZoom(svg, options));
            svg.dataset.panZoom = 'ready';
        } catch (error) {
            undo(svg);
            if (retriesLeft > 0) {
                setTimeout(() => attach(block, retriesLeft - 1), RETRY_MS);
            } else {
                // A diagram that cannot be zoomed is still a readable diagram.
                svg.dataset.panZoom = 'failed';
                svg.dataset.panZoomError = String(error && error.message);
                console.error('svg-pan-zoom: could not attach to a diagram', error);
            }
        }
    };

    const watch = (block) => {
        let timer;
        const settle = () => {
            clearTimeout(timer);
            timer = setTimeout(() => attach(block), SETTLE_MS);
        };
        new MutationObserver(settle).observe(block, { childList: true, subtree: true });
        if (block.querySelector('svg')) {
            settle();
        }
    };

    const start = () => document.querySelectorAll('pre.mermaid').forEach(watch);
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', start);
    } else {
        start();
    }
})();
