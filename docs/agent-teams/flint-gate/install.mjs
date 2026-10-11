// Apply only this team's reviewed native definitions inside this project.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
const root=fileURLToPath(new URL('../../../',import.meta.url));
process.chdir(root);
const base='docs/agent-teams/flint-gate';
const request=JSON.parse(fs.readFileSync(`${base}/team-request.json`));
const roles=request.team.roles, skills=[...new Set(roles.flatMap(r=>r.skills))];
const files=[],links=[];
for(const [target,dir,ext] of [['codex','.codex/agents','.toml'],['claude','.claude/agents','.md'],['copilot','.github/agents','.agent.md'],['kimi','.kimi-code/agents','.md'],['opencode','.opencode/agents','.md']]){
 for(const role of roles)files.push({source:`${base}/exports/${target}/${dir}/${role.id}${ext}`,destination:`${dir}/${role.id}${ext}`});
}
for(const role of roles)files.push({source:`${base}/exports/minimax/agents/${role.id}/agent.md`,destination:`.agent-team/minimax/agents/${role.id}/agent.md`});
for(const dir of ['.codex/skills','.claude/skills','.github/skills','.kimi-code/skills','.opencode/skills']){
 for(const skill of skills)links.push({destination:`${dir}/${skill}`,target:path.relative(dir,`.agents/skills/${skill}`)});
}
links.push({destination:'.agent-team/minimax/skills',target:'../../.agents/skills'});
const stat=p=>{try{return fs.lstatSync(p);}catch(e){if(e.code==='ENOENT')return null;throw e;}};
// Reject symlinked ancestors so a project-local path cannot write outside the checkout.
function safe(p){assert.ok(!path.isAbsolute(p)&&!p.split('/').includes('..'));let dir=path.dirname(p);while(dir!=='.'){const s=stat(dir);assert.ok(!s?.isSymbolicLink(),`Symlink ancestor: ${dir}`);dir=path.dirname(dir);}}
for(const f of files){safe(f.destination);const s=stat(f.destination);if(s)assert.ok(s.isFile()&&!s.isSymbolicLink()&&fs.readFileSync(f.source).equals(fs.readFileSync(f.destination)),`Existing definition differs: ${f.destination}`);}
for(const l of links){safe(l.destination);const s=stat(l.destination);if(s)assert.ok(s.isSymbolicLink()&&fs.readlinkSync(l.destination)===l.target,`Existing skill mapping differs: ${l.destination}`);assert.ok(fs.existsSync(path.resolve(path.dirname(l.destination),l.target)),`Missing skill: ${l.target}`);}
for(const f of files){fs.mkdirSync(path.dirname(f.destination),{recursive:true});if(!stat(f.destination))fs.copyFileSync(f.source,f.destination,fs.constants.COPYFILE_EXCL);}
for(const l of links){fs.mkdirSync(path.dirname(l.destination),{recursive:true});if(!stat(l.destination))fs.symlinkSync(l.target,l.destination,'dir');}
const sha=p=>createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const receipt={at:new Date().toISOString(),root,files:files.map(f=>({...f,sha256:sha(f.destination)})),links,stagedOnly:['deepseek','uar','bossfang','claude plugin alternative','kimi plugin alternative'],globalChanges:false,agentsStarted:false};
fs.writeFileSync(`${base}/evidence/project-installation.json`,JSON.stringify(receipt,null,2)+'\n');
console.log(JSON.stringify({installedDefinitions:files.length,skillLinks:links.length,agentsStarted:false}));
