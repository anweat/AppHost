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
 folder:'M3 6h7l2 3h9v12H3V6Z',activity:'M3 12h4l3-8 4 16 3-8h4',
 chevron:'m6 9 6 6 6-6',arrow:'M7 17 17 7M9 7h8v8',pause:'M9 6v12M15 6v12',plus:'M12 5v14M5 12h14',
 play:'M8 5v14l11-7L8 5Z',stop:'M7 7h10v10H7V7Z',restart:'M4 12a8 8 0 1 0 2.3-5.6M4 4v4h4',logs:'M4 6h16M4 12h16M4 18h10',
 info:'M12 11v6m0-10h.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z',search:'M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14Zm5-2 5 5',
 newgroup:'M3 6h7l2 3h9v12H3V6Zm9 5v6m-3-3h6',edit:'M4 20h4L19 9l-4-4L4 16v4Z',back:'M15 18l-6-6 6-6',
 moon:'M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5Z',command:'M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6Z',
 ungroup:'M3 6h7l2 3h9v12H3V6Zm6 9h6',expand:'M4 9V4h5M20 15v5h-5M20 9V4h-5M4 15v5h5'
};
const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.grid}"/></svg>`;
const escape = value => String(value).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
