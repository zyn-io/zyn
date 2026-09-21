import test from 'node:test';
import assert from 'node:assert/strict';
import { createState, advance, floor, totalValue, ITEMS } from '../src/model.js';

test('the demo uses only the standalone Shielded Byte card', () => {
  assert.equal(ITEMS.length, 1);
  assert.equal(ITEMS[0].id, 'shielded-byte-001');
  assert.equal(createState().selected, ITEMS[0].id);
  assert.throws(() => createState('owl-0057'), /Unknown item/);
});

test('seven beats preserve value, bind the rejected sale and complete the exit', () => {
  let s = createState();
  const total = totalValue(s);
  s = advance(s); assert.equal(s.a, 1000000); assert.equal(s.externalA, 1000000);
  s = advance(s); assert.equal(s.owner, 'A'); assert.equal(floor(s),500000); assert.equal(s.creator,500000);
  s = advance(s); assert.equal(s.owner,'B'); assert.equal(s.b,1394000); assert.equal(s.a,1594000); assert.equal(floor(s),501000);
  const before = {a:s.a,b:s.b,pool:s.pool,owner:s.owner};
  s = advance(s); assert.equal(s.step,3); assert.equal(s.blocked,true);
  assert.deepEqual({a:s.a,b:s.b,pool:s.pool,owner:s.owner},before);
  s = advance(s); assert.equal(s.owner,'A'); assert.equal(s.forced,true); assert.equal(floor(s),502000);
  s = advance(s); assert.equal(s.owner,'burned'); assert.equal(s.outstanding,5); assert.equal(s.redeemed,502000);
  s = advance(s); assert.equal(s.anchor.transitions.length,3); assert.equal(new Set(s.anchor.transitions).size,3);
  s = advance(s); assert.equal(s.step,7); assert.equal(s.externalA,1002000); assert.equal(s.a,988000);
  assert.equal(s.chainTxs,5); assert.equal(totalValue(s),total); assert.throws(()=>advance(s),/complete/);
});
test('each transition conserves integer value for every selectable piece', () => {
  for(const item of ITEMS){
    let s=createState(item.id); const total=totalValue(s);
    while(s.step<7){s=advance(s);assert.equal(totalValue(s),total);for(const key of ['a','b','externalA','pool','creator']){assert.ok(Number.isSafeInteger(s[key]));assert.ok(s[key]>=0);}}
  }
});
test('reset is deterministic and old states are not mutated', () => {
  const a=createState(); const snap=structuredClone(a); advance(a);assert.deepEqual(a,snap);assert.deepEqual(createState(),snap);
});
test('insufficient buyer funds cannot produce a partial trade',()=>{
  let s=advance(advance(createState()));s.b=0;const before=structuredClone(s);
  assert.throws(()=>advance(s),/funded/);assert.deepEqual(s,before);
});
