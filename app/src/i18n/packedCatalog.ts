import keysUrl from 'virtual:telegram-drive-locale-keys';

interface KeyTable {id:string;keys:string[];shared:string[]}

function readKeys(table: { format?: string; keys?: unknown; groups?: unknown }): string[] {
  if (table.format === 'td-keys-v2' && Array.isArray(table.keys)) return table.keys;
  if (table.format !== 'td-keys-v3' || !Array.isArray(table.groups)) throw new Error('Invalid language key table');
  return table.groups.flatMap(group => {
    if (!Array.isArray(group) || group.length < 2 || group.some(part => typeof part !== 'string')) {
      throw new Error('Invalid language key group');
    }
    const [prefix, ...names] = group;
    return names.map(name => prefix ? `${prefix}.${name}` : name);
  });
}
let keyRequest:Promise<KeyTable>|undefined;
async function keyTable(signal?:AbortSignal):Promise<KeyTable> {
  if(!keysUrl)throw new Error('Missing language key table');
  if(!keyRequest) {
    const request=(async()=>{
      const response=await fetch(keysUrl,{signal});if(!response.ok)throw new Error('Language key table unavailable');
      const packed = await response.json();
      const table: KeyTable = {id: packed.id, shared: packed.shared, keys: readKeys(packed)};
      if(!Array.isArray(table.shared)||table.shared.some(value=>typeof value!=='string')||typeof table.id!=='string'||!Array.isArray(table.keys)||new Set(table.keys).size!==table.keys.length||table.keys.some(key=>typeof key!=='string'||!key||key.split('.').some(part=>!part||['__proto__','prototype','constructor'].includes(part))))throw new Error('Invalid language key table');
      if(signal?.aborted)throw new Error('Language key request cancelled');
      return table;
    })();
    keyRequest=request;
    void request.catch(()=>{if(keyRequest===request)keyRequest=undefined;});
  }
  return keyRequest;
}

export async function unpackCatalog(value:unknown,signal?:AbortSignal,language?:string):Promise<Record<string,unknown>> {
  if(!value||typeof value!=='object'||Array.isArray(value))throw new Error('Invalid language resource');
  const packed=value as {format?:unknown;id?:unknown;values?:unknown;language?:unknown};
  if(packed.format===undefined&&import.meta.env.DEV)return value as Record<string,unknown>; // Development uses the canonical JSON.
  if(packed.format!=='td-locale-v2'||packed.language!==language||!Array.isArray(packed.values))throw new Error('Unsupported language resource');
  const table=await keyTable(signal);
  if(packed.id!==table.id||packed.values.length!==table.keys.length)throw new Error('Language key table mismatch');
  const resource:Record<string,unknown>=Object.create(null);
  for(let index=0;index<table.keys.length;index++) {
    const slot=packed.values[index];
    if(slot===null)continue;
    const translated=typeof slot==='number' && Number.isSafeInteger(slot) ? table.shared[slot] : slot;
    if(typeof translated!=='string')throw new Error('Invalid language value');
    const path=table.keys[index].split('.');
    let parent=resource;
    for(const name of path.slice(0,-1)) {
      if(parent[name]===undefined)parent[name]=Object.create(null);
      if(typeof parent[name]!=='object'||parent[name]===null)throw new Error('Conflicting language key');
      parent=parent[name] as Record<string,unknown>;
    }
    const name=path[path.length-1];if(parent[name]!==undefined)throw new Error('Conflicting language key');
    parent[name]=translated;
  }
  return resource;
}
