// The viewport picker: the widths a page is shown at (a phone, a tablet, a
// laptop…), a custom width, and optionally "fit" (the room there is). One
// markup and one change handler, shared by the mock-up's easel and the
// creation page's live preview — both keep their own width and their own
// resizing. Exposed as `window.Viewport`.
(() => {
  'use strict';

  const DEFAULT_WIDTH = 1280;
  const MIN_WIDTH = 320;
  const MAX_WIDTH = 3000;
  const PRESETS = [['Mobile', 390], ['Tablet', 768], ['Laptop', 1280], ['Large', 1536]];
  const FIT = 'fit';
  const CUSTOM = '0';

  const clamp = (w) => Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.round(w)));

  // The picker's markup; `fit` adds the "Fit" choice. `sync` then sets it.
  function markup({ fit = false } = {}) {
    const options = (fit ? [`<option value="${FIT}">Fit</option>`] : [])
      .concat(PRESETS.map(([n, w]) => `<option value="${w}">${n} · ${w}</option>`))
      .concat(`<option value="${CUSTOM}">custom</option>`)
      .join('');
    const label = '<label class="mk-vp" title="the width the page is shown at"><select class="mk-presets" aria-label="viewport">'
      + options + '</select><input class="mk-width" type="number" min="' + MIN_WIDTH + '" max="' + MAX_WIDTH + '" step="10" aria-label="width in pixels" hidden></label>';
    return label;
  }

  // Puts the picker in line with the width: the preset it matches, else "custom" and its field.
  function sync(root, width) {
    const presets = root.querySelector('.mk-presets');
    const field = root.querySelector('.mk-width');
    if (!presets || !field) return;
    if (width === null) presets.value = FIT;
    else presets.value = PRESETS.some(([, w]) => w === width) ? String(width) : CUSTOM;
    if (width !== null) field.value = width;
    field.hidden = presets.value !== CUSTOM;
  }

  // A `change` event on the picker: `apply(width)` gets a number, or null for "fit".
  // Returns whether the event was the picker's.
  function change(e, apply) {
    const root = e.target.closest('.mk-vp');
    if (!root) return false;
    if (e.target.classList.contains('mk-presets')) {
      if (e.target.value === FIT) apply(null);
      else if (e.target.value !== CUSTOM) apply(Number(e.target.value));
      else { const field = root.querySelector('.mk-width'); field.hidden = false; field.focus(); }
      return true;
    }
    if (e.target.classList.contains('mk-width')) { apply(clamp(Number(e.target.value) || DEFAULT_WIDTH)); return true; }
    return false;
  }

  window.Viewport = { DEFAULT_WIDTH, MIN_WIDTH, MAX_WIDTH, PRESETS, clamp, markup, sync, change };
})();
