import {version} from './version.js';
document.querySelector('#version').textContent = version;
if (import.meta.hot) import.meta.hot.accept('./version.js', module => {
  document.querySelector('#version').textContent = module.version;
  document.querySelector('#hmr').textContent = 'HMR received without navigation';
});
