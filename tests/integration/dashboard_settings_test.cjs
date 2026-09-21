// Execute the shipped dashboard script against a small DOM and controlled HTTP
// responses. No microphone, browser navigation, or personal settings are used.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const html = fs.readFileSync(path.join(__dirname, '../../src-tauri/assets/dashboard.html'), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1]
  .replace(/\npoll\(\);\s*checkSetup\(\);\s*setInterval\(poll, 5000\);/, '');
const nodes = new Map();
const node = id => {
  if (!nodes.has(id)) nodes.set(id, {value:'', checked:false, textContent:'', style:{},
    classList:{add(){},remove(){}}, addEventListener(){}});
  return nodes.get(id);
};
let settings = {cleanup_mode:'faithful', stutter_correction:true, retain_audio_for_retry:false,
  retry_audio_seconds:120, chimes_enabled:true, hud_enabled:true};
let failSave = false;
let pendingPoll;
const requests = [];
const snapshot = () => ({settings:{...settings}, runtime:{effective_settings:{...settings}}});
const context = vm.createContext({window:{}, document:{getElementById:node,
  querySelector:node, querySelectorAll:()=>[], createElement:()=>node('created'), body:node('body')},
  setInterval(){}, console, fetch:async (url, options={}) => {
    if (url === '/api/settings') {
      requests.push(JSON.parse(options.body));
      if (failSave) return {ok:false,json:async()=>({error:'disk unavailable'})};
      Object.assign(settings,JSON.parse(options.body));
      return {ok:true,json:async()=>({settings:{...settings}})};
    }
    if (url === '/api/status' && pendingPoll) return pendingPoll;
    return {ok:true,json:async()=>url==='/api/status'?snapshot():{items:[]}};
  }});
vm.runInContext(script,context); // Also checks syntax of the entire shipped script.
context.state=snapshot();
vm.runInContext('render(state)',context);
async function change(mode) {
  node('cleanup-mode').value=mode;
  context.window.settingsDirty=true;
  await node('cleanup-mode').onchange();
  await new Promise(resolve=>setImmediate(resolve));
}
(async()=>{
  // Selecting Polished alone persists, confirms active state, and clears dirty.
  await change('polished');
  assert.deepEqual(requests[0],{cleanup_mode:'polished'});
  assert.equal(node('cleanup-active').textContent,'Active: Polished');
  assert.equal(settings.cleanup_mode,'polished');
  assert.equal(context.window.settingsDirty,false);
  // Mode-only autosave must leave unrelated pending edits in the form.
  node('retry-seconds').value='300';
  context.window.settingsDirty=true;
  await change('faithful');
  assert.equal(node('retry-seconds').value,'300');
  assert.equal(settings.retry_audio_seconds,120);
  assert.equal(context.window.settingsDirty,true);
  await node('save-settings').onclick();
  assert.equal(settings.retry_audio_seconds,300);
  // A pre-save HTTP response arriving late cannot revert the selected mode.
  const stale=snapshot(); let resolvePoll;
  pendingPoll=new Promise(resolve=>{resolvePoll=resolve});
  const oldPoll=vm.runInContext('poll()',context);
  pendingPoll=null;
  await change('polished');
  resolvePoll({json:async()=>stale}); await oldPoll;
  assert.equal(node('cleanup-mode').value,'polished');
  assert.equal(node('cleanup-active').textContent,'Active: Polished');
  // Failed writes restore the actual mode and show a failure, not confirmation.
  failSave=true;
  await change('faithful');
  assert.equal(node('cleanup-mode').value,'polished');
  assert.match(node('settings-error').textContent,/Not saved: disk unavailable/);
  assert.equal(settings.cleanup_mode,'polished');
  console.log('4 dashboard settings scenarios passed');
})().catch(error=>{console.error(error);process.exitCode=1});
