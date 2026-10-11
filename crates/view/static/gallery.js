// The gallery: a grid of tiles that only scrolls down, shared by the
// design system's kit and the mock-up's micro-frontends. A tile is a
// preview at a fixed height — a frame loaded while it is on the screen or a
// screen away, let go further — and a foot; nothing moves as frames come
// in. The finder (every word typed, accents and case ignored, `/` to reach
// it) and the chips stay above it; a list changed by them starts again from
// the top. The pane decides what a tile shows and what opening one does.
//
//   const gallery = HarnessGallery.create(host, {
//     placeholder, chips: [[id, label, (item) => bool]], words: (item) => [text],
//     key: (item) => name, src: (item) => url | '', body: (item) => html, foot: (item) => html,
//     empty: (total, find, chipLabel) => html, onOpen: (item) => {}, onFrame: (frame) => {},
//   });
//   gallery.draw(items); gallery.list(); gallery.pause(true); gallery.top(); gallery.scrollTo(y);
(() => {
  'use strict';

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const plain = (text) => String(text || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

  function create(host, opts) {
    host.innerHTML = '<div class="kt-gallery"><div class="kt-gallery-top">'
      + `<input type="search" class="kt-find" maxlength="80" placeholder="${esc(opts.placeholder || 'Find…')}" aria-label="find" title="Press / to come here">`
      + '<span class="kt-count"></span><div class="kt-chips" role="group" aria-label="show"></div></div>'
      + '<div class="kt-grid"></div><div class="mk-empty kt-none" hidden></div></div>';
    const scroller = host.querySelector('.kt-gallery');
    const grid = host.querySelector('.kt-grid');
    const find = host.querySelector('.kt-find');
    const st = { find: '', chip: (opts.chips && opts.chips[0] && opts.chips[0][0]) || 'all', items: [], paused: false, seen: null };

    const chipOf = () => (opts.chips || []).find(([id]) => id === st.chip);
    function matches(item) {
      const words = plain(st.find).split(/\s+/).filter(Boolean);
      const hay = (opts.words ? opts.words(item) : []).flatMap((w) => [plain(w), plain(w).replace(/[^a-z0-9]/g, '')]);
      if (!words.every((w) => hay.some((h) => h.includes(w)))) return false;
      const chip = chipOf();
      return !chip || !chip[2] || chip[2](item);
    }
    const list = () => st.items.filter(matches);

    // A preview is a whole document, its scripts running: only the tiles on
    // the screen, and a screen above and below, hold one. A gallery paused —
    // hidden behind one item — lets nothing go.
    function watch(frame) {
      if (!window.IntersectionObserver) { if (!frame.dataset.loaded) { frame.dataset.loaded = frame.dataset.src; frame.src = frame.dataset.src; } return; }
      if (!st.seen) {
        st.seen = new IntersectionObserver((entries) => {
          if (st.paused) return;
          entries.forEach((entry) => {
            const f = entry.target;
            if (entry.isIntersecting) {
              if (f.dataset.loaded !== f.dataset.src) { f.dataset.loaded = f.dataset.src; f.src = f.dataset.src; }
            } else if (f.dataset.loaded) {
              delete f.dataset.loaded;
              f.src = 'about:blank';
            }
          });
        }, { root: scroller, rootMargin: '100% 0px' });
      }
      st.seen.observe(frame);
    }

    function draw(items) {
      if (items) st.items = items;
      const shown = list();
      host.querySelector('.kt-count').textContent = `${shown.length} / ${st.items.length}`;
      host.querySelector('.kt-chips').innerHTML = (opts.chips || []).map(([id, label]) => `<button type="button" class="kt-chip${st.chip === id ? ' on' : ''}" data-chip="${esc(id)}">${esc(label)}</button>`).join('');
      const have = new Map([...grid.children].map((tile) => [tile.dataset.card, tile]));
      let before = null;
      shown.forEach((item) => {
        const key = opts.key(item);
        const src = opts.src ? opts.src(item) : '';
        let tile = have.get(key);
        have.delete(key);
        // A tile that changes from a preview to none, or back, is made again.
        if (tile && !!tile.querySelector('iframe') !== !!src) { if (st.seen) st.seen.unobserve(tile.querySelector('iframe') || tile); tile.remove(); tile = null; }
        if (!tile) {
          tile = document.createElement('article');
          tile.className = 'kt-tile';
          tile.dataset.card = key;
          if (opts.pin) tile.dataset.pin = opts.pin(item);
          tile.tabIndex = 0;
          tile.innerHTML = `<div class="kt-tile-preview">${src ? `<iframe class="mk-preview" tabindex="-1" scrolling="no" title="${esc(key)}"></iframe>` : '<div class="kt-tile-body"></div>'}</div><div class="kt-tile-foot"></div>`;
          const frame = tile.querySelector('iframe');
          if (frame && opts.onFrame) frame.addEventListener('load', () => opts.onFrame(frame));
        }
        const frame = tile.querySelector('iframe');
        if (frame && frame.dataset.src !== src) {
          frame.dataset.src = src;
          // Shown already: the new version at once; else when it comes on the screen.
          if (frame.dataset.loaded) { frame.dataset.loaded = src; frame.src = src; }
        }
        if (!frame && opts.body) {
          const body = opts.body(item);
          if (tile.dataset.body !== body) { tile.dataset.body = body; tile.querySelector('.kt-tile-body').innerHTML = body; }
        }
        const foot = opts.foot(item);
        if (tile.dataset.foot !== foot) { tile.dataset.foot = foot; tile.querySelector('.kt-tile-foot').innerHTML = foot; }
        const next = before ? before.nextSibling : grid.firstChild;
        if (tile !== next) grid.insertBefore(tile, next);
        if (frame) watch(frame);
        before = tile;
      });
      have.forEach((tile) => { const f = tile.querySelector('iframe'); if (f && st.seen) st.seen.unobserve(f); tile.remove(); });
      const none = host.querySelector('.kt-none');
      none.hidden = shown.length > 0;
      if (!shown.length) none.innerHTML = opts.empty(st.items.length, st.find, st.chip !== 'all' && chipOf() ? chipOf()[1] : '');
    }
    // The list changed by the finder or a chip: the gallery starts again from the top.
    function refind() { draw(); scroller.scrollTop = 0; }

    find.addEventListener('input', () => { st.find = find.value; refind(); });
    host.addEventListener('click', (e) => {
      const chip = e.target.closest('.kt-chip');
      if (chip) { st.chip = chip.dataset.chip; refind(); return; }
      const tile = e.target.closest('.kt-tile');
      if (tile && !e.target.closest('button, a, input, select')) {
        const item = st.items.find((x) => opts.key(x) === tile.dataset.card);
        if (item) opts.onOpen(item);
      }
    });
    // `/` reaches the finder; Enter on a tile opens it — while the gallery is shown.
    const onKey = (e) => {
      if (!scroller.isConnected || st.paused || !scroller.offsetParent) return;
      const typing = e.target && (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA' || e.target.tagName === 'SELECT' || e.target.isContentEditable);
      if (e.key === '/' && !typing) { e.preventDefault(); find.focus(); return; }
      if (e.key === 'Enter' && e.target && e.target.classList && e.target.classList.contains('kt-tile') && host.contains(e.target)) {
        const item = st.items.find((x) => opts.key(x) === e.target.dataset.card);
        if (item) opts.onOpen(item);
      }
    };
    document.addEventListener('keydown', onKey);

    return {
      draw,
      list,
      frames: () => [...grid.querySelectorAll('iframe.mk-preview')],
      // Hidden behind one item: its frames are kept, its finder sleeps.
      pause(on) { st.paused = !!on; },
      top: () => scroller.scrollTop,
      scrollTo(y) { scroller.scrollTop = y || 0; },
      destroy() {
        document.removeEventListener('keydown', onKey);
        if (st.seen) st.seen.disconnect();
        host.innerHTML = '';
      },
    };
  }

  window.HarnessGallery = { create };
})();
