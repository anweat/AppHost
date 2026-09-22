'use strict';
// Shared inline assets; no network requests or icon package.
const paths = {
 book:'M4 4h7c2 0 2 2 2 2s0-2 2-2h5v16h-5c-2 0-2 1-2 1s0-1-2-1H4V4Zm9 2v15',
 camera:'M3 7h5l2-3h4l2 3h5v13H3V7Zm13 6a4 4 0 1 1-8 0 4 4 0 0 1 8 0Z',
 canvas:'M3 3h18v18H3V3Zm0 13 6-6 4 4 3-3 5 5M16 7h.01',
 wave:'M3 10v4m4-8v12m5-16v20m5-16v12m4-8v4',
 note:'M5 3h14v18H5V3Zm4 5h6m-6 4h6m-6 4h4',
 grid:'M3 3h7v7H3V3Zm11 0h7v7h-7V3ZM3 14h7v7H3v-7Zm11 0h7v7h-7v-7Z',
 clock:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-6v6l4 2',
 sliders:'M6 3v7m0 4v7m6-18v3m0 4v11m6-18v11m0 4v3M3 10h6v4H3v-4Zm6-4h6v4H9V6Zm6 8h6v4h-6v-4',
 folder:'M3 6h7l2 3h9v12H3V6Z',activity:'M3 12h4l3-8 4 16 3-8h4'
};
const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.grid}"/></svg>`;
const escape = value => String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
