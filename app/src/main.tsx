import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import i18n,{ensureLanguageResource,i18nInitialized} from './i18n';
import bootstrapCopy from 'virtual:telegram-drive-bootstrap-copy';
import {resolveLanguagePreference} from './i18n/resolveLanguage';
import { loadAppStore } from './services/appStore';
import { reportCrash } from './services/crashTelemetry';

window.onerror = function (message, source, lineno, colno, error) {
  console.error("Global JS Error:", message, "at", source, lineno + ":" + colno, error?.stack || error);
  reportCrash(error || new Error(String(message)), 'window');
  return false;
};

window.addEventListener("unhandledrejection", function (event) {
  console.error("Unhandled Promise Rejection:", event.reason, event.reason?.stack || event.reason);
  reportCrash(event.reason, 'promise');
});

const container=document.getElementById('root') as HTMLElement;
let language=resolveLanguagePreference('system');
let finished=false;
const controller=new AbortController();

function showBootstrap(error=false) {
  const copy=bootstrapCopy[language];
  document.documentElement.lang=language;
  document.documentElement.dir=copy.dir;
  const section=document.createElement('section');
  section.className='flex h-screen items-center justify-center bg-app-canvas p-8 text-app-text';
  section.dataset.languageBootstrap=error?'error':'loading';
  section.setAttribute('role',error?'alert':'status');
  const content=document.createElement('div');content.className='max-w-sm text-center';
  const title=document.createElement('h1');title.className='text-lg font-semibold';title.textContent=copy.title;
  const message=document.createElement('p');message.className='mt-3 text-sm text-app-text-secondary';message.textContent=error?copy.error:copy.loading;
  content.append(title,message);
  if(error) {
    const retry=document.createElement('button');retry.type='button';retry.className='quiet-control mt-4 px-4 py-2';retry.textContent=copy.retry;
    retry.onclick=()=>window.location.reload();content.append(retry);
  }
  section.append(content);container.replaceChildren(section);
}

// Read only the saved language; preference errors are still handled by the
// application's normal persistence workflow. A warming native store cannot
// hold the independent local-resource loading screen indefinitely.
async function savedLanguage() {
  let timer:number|undefined;
  try {
    return await Promise.race([
      loadAppStore('settings.json').then(store=>store.get<{language?:Parameters<typeof resolveLanguagePreference>[0]}>('settings')).then(settings=>typeof settings?.language==='string'?settings.language:undefined).catch(()=>undefined),
      new Promise<undefined>(done=>{timer=window.setTimeout(()=>done(undefined),700);}),
    ]);
  } finally {window.clearTimeout(timer);}
}

showBootstrap();
const deadline=window.setTimeout(()=>{
  controller.abort();
  if(!finished){finished=true;showBootstrap(true);}
},8_000);
void (async()=>{
  const preference=savedLanguage();
  await i18nInitialized;
  await ensureLanguageResource('en',controller.signal);
  language=resolveLanguagePreference((await preference)||'system');
  if(controller.signal.aborted)throw new Error('Language bootstrap cancelled');
  showBootstrap();
  await ensureLanguageResource(language,controller.signal);
  await i18n.changeLanguage(language);
  if(finished||controller.signal.aborted)return;
  finished=true;window.clearTimeout(deadline);container.replaceChildren();
  ReactDOM.createRoot(container).render(<React.StrictMode><App /></React.StrictMode>);
})().catch(()=>{
  window.clearTimeout(deadline);
  if(!finished){finished=true;controller.abort();showBootstrap(true);}
});
