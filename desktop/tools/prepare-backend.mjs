import { spawnSync, execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
// Native builds only. Cross builds must name their target explicitly.
const flag = process.argv.indexOf('--target');
const target = flag >= 0 ? process.argv[flag + 1] : execFileSync('rustc', ['--print', 'host-tuple'], { encoding: 'utf8' }).trim();
if (!target || !/^[a-zA-Z0-9_-]+$/.test(target)) throw new Error('Invalid build target');
const debug = process.argv.includes('--debug') || process.env.TAURI_ENV_DEBUG === 'true';
const result = spawnSync('cargo', ['build', '--locked', '--bin', 'apphost', '--target', target, ...(debug ? [] : ['--release'])], { cwd: root, stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status ?? 1);
const ext = target.includes('windows') ? '.exe' : '';
const directory = resolve(root, 'desktop/src-tauri/binaries');
mkdirSync(directory, { recursive: true });
copyFileSync(resolve(root, 'target', target, debug ? 'debug' : 'release', `apphost${ext}`), resolve(directory, `apphost-${target}${ext}`));
