import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source=readFileSync(new URL('../src/scan-consent.ts',import.meta.url),'utf8');
const js=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2020}}).outputText;
const {scanSpendVisible,acceptSpendBatch}=await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
test('local source visibility is independent of query-account consent',()=>{
 assert.equal(scanSpendVisible({id:'minimax',sources:['claude']},{claude:['/chosen/logs']},false),true);
 assert.equal(scanSpendVisible({id:'minimax',sources:['claude']},{},true),false);
});
test('only separately authorized Cursor can use empty source provenance',()=>{
 assert.equal(scanSpendVisible({id:'cursor',sources:[]},{},true),true);
 assert.equal(scanSpendVisible({id:'cursor',sources:[]},{},false),false);
 assert.equal(scanSpendVisible({id:'minimax',sources:[]},{},true),false);
});
test('late spend cannot cross a consent epoch or replace a newer refresh',()=>{
 assert.equal(acceptSpendBatch(1,2,10,9),false);
 assert.equal(acceptSpendBatch(2,2,8,9),false);
 assert.equal(acceptSpendBatch(2,2,10,9),true);
});
