const fs = require('node:fs'), path = require('node:path');
const root = path.resolve(__dirname, '..', '.github/workflows');
let count = 0;
for (const name of fs.readdirSync(root).filter(name => /\.ya?ml$/.test(name))) {
  const text = fs.readFileSync(path.join(root, name), 'utf8');
  for (const match of text.matchAll(/^\s*(?:-\s*)?uses:\s*([^\s#]+)/gm)) {
    const action = match[1].replace(/^['"]|['"]$/g, '');
    if (!action.startsWith('./') && !/^[\w.-]+\/[\w./-]+@[a-f0-9]{40}$/.test(action)) throw new Error(`${name}: third-party action is not pinned to a full commit: ${action}`);
  }
  count++;
}
const assurance = fs.readFileSync(path.join(root, 'dependency-assurance.yml'), 'utf8');
if (!/\n  schedule:/.test(assurance) || !/\n  baseline-expiry:/.test(assurance)) throw new Error('Scheduled baseline-expiry gate must remain');
const release = fs.readFileSync(path.join(root, 'release.yml'), 'utf8');
for (const platform of ['macos', 'windows']) if (!release.includes(`if: env.PLATFORM_SIGNING == '${platform}'`)) throw new Error(`Missing conditional ${platform} signature gate`);
const pages = fs.readFileSync(path.join(root, 'pages.yml'), 'utf8');
if (/^  release:/m.test(pages)) throw new Error('Pages must not deploy directly from a release tag; its protected environment permits only main');
const build = pages.split('\n  build:\n')[1]?.split('\n  deploy:\n')[0] ?? '';
const deploy = pages.split('\n  deploy:\n')[1] ?? '';
if (!build.includes('actions/upload-pages-artifact@') || !deploy.includes('actions/deploy-pages@')
  || build.includes('actions/deploy-pages@') || deploy.includes('actions/upload-pages-artifact@')
  || !/^    needs: build$/m.test(deploy)) throw new Error('Pages must upload its artifact in a completed build job before deploying');
if (![build, deploy].every(job => /^    if: github\.ref == 'refs\/heads\/main'$/m.test(job))) throw new Error('Pages jobs must be restricted to main');
if (/^concurrency:/m.test(pages) || /^    concurrency:/m.test(build) || !/^    concurrency:\n      group: pages\n      cancel-in-progress: true$/m.test(deploy)) throw new Error('Pages concurrency must apply only to the permitted deployment job');
const refresh = release.split('\n  refresh-product-page:\n')[1];
const stableMainDispatch = refresh
  && /^    needs: publish-release$/m.test(refresh)
  && /^    if: \$\{\{ !contains\(github\.ref_name, '-'\) \}\}$/m.test(refresh)
  && /^    permissions:\n      actions: write\n    steps:/m.test(refresh)
  && refresh.includes('github.rest.actions.createWorkflowDispatch')
  && refresh.includes("workflow_id: 'pages.yml'")
  && refresh.includes("ref: 'main'");
if (!stableMainDispatch) throw new Error('Stable desktop publication must explicitly dispatch the protected Pages workflow on main with only actions:write');
console.log(`[workflow-pins] ${count} workflow action pins and scheduled/conditional/deployment protections passed; workflow execution remains UNRUN.`);
