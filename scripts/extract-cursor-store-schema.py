#!/usr/bin/env python3
"""Extract protocol field metadata from a deliberately selected native CLI build.

This reads source text only. It never imports or executes the provider code.
The generated metadata is private to the versioned Cursor store decoder.
"""
import argparse, hashlib, re, json
from pathlib import Path
parser=argparse.ArgumentParser(description='Read compact protobuf descriptors without executing native CLI code')
parser.add_argument('source',type=Path)
parser.add_argument('--version',required=True)
parser.add_argument('--output',type=Path,required=True)
cli=parser.parse_args()
raw=cli.source.read_bytes()
source=raw.decode('utf-8')
starts=list(re.finditer(r'"([^"\n]+\.js)"\(([A-Za-z0-9_$,]+)\)\{',source))
modules={}
for i,m in enumerate(starts):
 if len(m[2].split(','))!=3: continue
 text=source[m.end():starts[i+1].start() if i+1<len(starts) else len(source)]
 args=m[2].split(','); require=args[2]; exp=args[1]
 exports={}
 match=re.search(re.escape(require)+r'\.d\('+re.escape(exp)+r',\{(.*?)\}\)',text)
 if match:
  exports=dict(re.findall(r'([\w$]+):\(\)=>((?:[\w$]+\.)?[\w$]+)',match[1]))
 imports=dict(re.findall(r'([\w$]+)='+re.escape(require)+r'\("([^"\n]+)"\)',text))
 classes={}; bases={}
 cs=list(re.finditer(r'class ([\w$]+) extends ([\w$.]+)\{',text))
 for j,c in enumerate(cs):
  body=text[c.end():cs[j+1].start() if j+1<len(cs) else len(text)]
  pre=re.search(r'static \$p\(\)\{return\s*("[^"\n]+"|[\w$]+)\}',body)
  if pre:
   val=pre[1]
   if not val.startswith(chr(34)):
    v=re.search(r'\b'+re.escape(val)+r'=('+chr(34)+r'[^'+chr(34)+r']+'+chr(34)+r')',text)
    val=v[1] if v else chr(34)+chr(34)
   bases[c[1]]=json.loads(val)
  desc=re.search(r'static \$\(\)\{return\[("(?:[^"\\]|\\.)*")([^\]]*)\]\}',body)
  if desc:
   d=json.loads(desc[1]); refs=[v.strip() for v in desc[2].split(',')[1:]]
   classes[c[1]]={'name': d.split('|')[0], 'fields':d.split('|')[1:], 'refs':refs,'base':c[2]}
 modules[m[1]]={'text':text,'exports':exports,'imports':imports,'classes':classes,'bases':bases}

def resolve(module,ref):
 if '.' in ref:
  alias,name=ref.split('.',1)
  target=modules[module]['imports'].get(alias)
  if not target or target not in modules:return ('unknown',module,ref)
  exported=modules[target]['exports'].get(name)
  if not exported:return ('unknown',target,ref)
  return resolve(target,exported)
 cls=modules[module]['classes'].get(ref)
 if cls:
  base=cls['base']; prefix=modules[module]['bases'].get(base,'')
  return ('message',module,ref,prefix+cls['name'])
 # Compact enum references are numeric in our canonical JSON projection.
 if re.search(re.escape(ref)+r'=\(0,[\w$]+\.QT\)',modules[module]['text']):return ('enum',module,ref)
 # Generated google messages use the ordinary Buf runtime rather than compact descriptors.
 pat=rf'{re.escape(ref)}\.typeName="([^"\n]+)"'
 special=re.search(pat,modules[module]['text'])
 if special:return ('special',module,ref,special[1])
 return ('unknown',module,ref)

entry='../proto/dist/generated/agent/v1/agent_pb.js'
root=next(k for k,v in modules[entry]['classes'].items() if v['name']=='ToolCall')
seen={}; origins={}; missing=set(); enums=set(); special=set()
def walk(module,ref):
 result=resolve(module,ref)
 if result[0]!='message':
  (enums if result[0]=='enum' else special if result[0]=='special' else missing).add(result)
  return result
 _,mod,var,name=result
 if name in seen:
  if origins[name] != (mod,var): raise ValueError('Conflicting message identity: '+name)
  return result
 origins[name]=(mod,var)
 c=modules[mod]['classes'][var];seen[name]={'descriptor':c['fields'],'refs':[]}
 for f in c['refs']:seen[name]['refs'].append(walk(mod,f))
 return result
walk(entry,root)
if missing:
 raise ValueError('Unresolved generated type references: '+repr(sorted(missing)))
if {item[3] for item in special}!={'google.protobuf.Value','google.protobuf.Struct'}:
 raise ValueError('Unexpected external message type')
messages={}
for name, definition in sorted(seen.items()):
 refs=['enum' if ref[0]=='enum' else ref[3] for ref in definition['refs']]
 messages[name]={'fields':definition['descriptor'],'refs':refs}
artifact={'version':cli.version,'sourceSha256':hashlib.sha256(raw).hexdigest(),'messages':messages}
cli.output.write_text(json.dumps(artifact,ensure_ascii=True,indent=2)+'\n')
