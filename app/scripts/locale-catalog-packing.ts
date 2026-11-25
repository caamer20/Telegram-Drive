import {readFile,readdir} from 'node:fs/promises';
import {resolve,basename} from 'node:path';
import {createHash} from 'node:crypto';
import type {Plugin} from 'vite';

const virtualId='virtual:telegram-drive-locale-keys';
const resolvedId='\0'+virtualId;
const bootstrapId='virtual:telegram-drive-bootstrap-copy';
const resolvedBootstrap='\0'+bootstrapId;

// Source catalogs remain ordinary, reviewable JSON. Shipped catalogs share the
// key table so keys are stored once, while every translated value stays intact.
export function localeCatalogPacking():Plugin {
  let building=false;
  let root='';
  let keys:string[]=[];
  let id='';
  let shared: string[] = [];
  const stringIndices = new Map<string, number>();
  let reference:string|undefined;
  const catalogs=new Map<string,Record<string,string>>();
  function flatten(value:Record<string,unknown>,prefix='',output:Record<string,string>={}):Record<string,string> {
    for(const [name,entry] of Object.entries(value)) {
      if(['__proto__','constructor','prototype'].includes(name)||name.includes('.'))throw new Error('Invalid locale resource key');
      const key=prefix?prefix+'.'+name:name;
      if(typeof entry==='string')output[key]=entry;
      else if(entry&&typeof entry==='object'&&!Array.isArray(entry))flatten(entry as Record<string,unknown>,key,output);
      else throw new Error(`Invalid locale resource value: ${key}`);
    }
    return output;
  }
  return {
    name:'lossless-local-translation-data',enforce:'pre',
    configResolved(config){building=config.command==='build';root=config.root;},
    async buildStart(){
      if(!building)return;
      catalogs.clear();reference=undefined;
      const directory=resolve(root,'src/i18n/locales');
      for(const filename of (await readdir(directory)).filter(name=>name.endsWith('.json')).sort()) {
        const path=resolve(directory,filename);this.addWatchFile(path);
        catalogs.set(filename,flatten(JSON.parse(await readFile(path,'utf8'))));
      }
      keys=[...new Set([...catalogs.values()].flatMap(catalog=>Object.keys(catalog)))].sort();
      // A repeated translated value is stored once across the shipped catalogs.
      // Include an entry only when replacing its occurrences pays for its slot.
      const counts = new Map<string, number>();
      for (const catalog of catalogs.values()) {
        for (const value of Object.values(catalog)) counts.set(value, (counts.get(value) ?? 0) + 1);
      }
      shared = [...counts].filter(([value, count]) => count > 1 && (Buffer.byteLength(JSON.stringify(value)) - 5) * (count - 1) > 5).map(([value]) => value);
      stringIndices.clear();
      shared.forEach((value, index) => stringIndices.set(value, index));
      id=createHash('sha256').update(JSON.stringify([keys, shared])).digest('hex');
      // Adjacent keys share their namespace. Keep sorted key order so every
      // locale value and the integrity digest still refer to the same slot.
      const groups: string[][] = [];
      for (const key of keys) {
        const split = key.lastIndexOf('.');
        const prefix = split < 0 ? '' : key.slice(0, split);
        const name = key.slice(split + 1);
        const previous = groups[groups.length - 1];
        if (previous && previous[0] === prefix) previous.push(name);
        else groups.push([prefix, name]);
      }
      reference=this.emitFile({type:'asset',name:'translation-keys.json',source:JSON.stringify({format:'td-keys-v3',id,groups,shared})});
    },
    resolveId(value){if(value===virtualId)return resolvedId;if(value===bootstrapId)return resolvedBootstrap;},
    async load(value){
      if(value===resolvedBootstrap) {
        const directory=resolve(root,'src/i18n/locales');
        const copy:Record<string,unknown>={};
        for(const filename of (await readdir(directory)).filter(name=>name.endsWith('.json')).sort()) {
          const path=resolve(directory,filename);this.addWatchFile(path);
          const catalog=building?catalogs.get(filename)!:flatten(JSON.parse(await readFile(path,'utf8')));
          const language=filename.slice(0,-5);
          const labels=['common.app_title','common.loading','common.language_load_failed','common.retry'].map(key=>catalog[key]);
          if(labels.some(label=>typeof label!=='string'||!label))throw new Error(`Missing bootstrap copy: ${language}`);
          copy[language]={title:labels[0],loading:labels[1],error:labels[2],retry:labels[3],dir:['ar','fa-IR','ur-PK'].includes(language)?'rtl':'ltr'};
        }
        return `export default ${JSON.stringify(copy)};`;
      }
      if(value===resolvedId)return building?`export default import.meta.ROLLUP_FILE_URL_${reference};`:'export default null;';
      if(!building||!/[\\/]src[\\/]i18n[\\/]locales[\\/][^\\/]+\.json\?url$/.test(value))return;
      const filename=basename(value.slice(0,-4));
      const catalog=catalogs.get(filename);if(!catalog)throw new Error(`Unregistered language catalog: ${filename}`);
      const asset=this.emitFile({type:'asset',name:filename,source:JSON.stringify({format:'td-locale-v2',language:filename.slice(0,-5),id,values:keys.map(key => stringIndices.get(catalog[key]) ?? catalog[key] ?? null)})});
      return `export default import.meta.ROLLUP_FILE_URL_${asset};`;
    },
  };
}
