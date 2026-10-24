// Zyn Pay's wire format, held to the bytes the Rust ledger produces.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

import { hex, unhex } from '../src/object.js';
import * as m from '../src/market.js';

const here = dirname(fileURLToPath(import.meta.url));
const v = JSON.parse(readFileSync(join(here, 'object-vectors.json'), 'utf8')).market;
const W = (n) => BigInt(n) * m.WAD;

const ZEC = unhex(v.zec), OWNER = unhex(v.owner), NONCE = unhex(v.nonce);
const PAYEE = new Uint8Array(32).fill(0x9a), REFERENCE = new Uint8Array(32).fill(0x5e);
const FIRST_DUE = 1_793_491_200;
const funding = {
  inputs: [
    { commitment: new Uint8Array(32).fill(0xc1), nullifier: new Uint8Array(32).fill(0xd1) },
    { commitment: new Uint8Array(32).fill(0xc2), nullifier: new Uint8Array(32).fill(0xd2) },
  ],
  change: [{
    note: { asset: ZEC, amount: W(2), owner: OWNER, nonce: unhex(v.change_nonce) },
    enc: { epk: unhex(v.change_epk), ciphertext: unhex(v.change_ciphertext) },
  }],
};

test('Zyn Pay: programs and ids match the ledger', () => {
  assert.equal(m.MARKET_PROGRAM.payCreate, v.program_pay_create);
  assert.equal(m.MARKET_PROGRAM.payManage, v.program_pay_manage);
  assert.equal(m.MARKET_PROGRAM.payOnce, v.program_pay_once);
  assert.equal(m.MARKET_PROGRAM.payRefund, v.program_pay_refund);
  const schedule = m.scheduleId(OWNER, NONCE);
  assert.equal(hex(schedule), v.pay_schedule_id);
  const payment = m.payPaymentId(schedule, 0);
  assert.equal(hex(payment), v.pay_payment_id_0);
  assert.equal(hex(m.payPaymentId(schedule, 1)), v.pay_payment_id_1);
  assert.equal(hex(m.payOnceId(OWNER, NONCE)), v.pay_once_id);
  assert.equal(hex(m.refundSource(payment, 0)), v.pay_refund_source_0);
  assert.equal(hex(m.cancelSource(schedule)), v.pay_cancel_source);
  assert.equal(m.MIN_PAY_AMOUNT, 10n ** 15n);
});

test('Zyn Pay: every holder intent matches the ledger byte for byte', () => {
  const schedule = m.scheduleId(OWNER, NONCE);
  const payment = m.payPaymentId(schedule, 0);
  const create = { payer: OWNER, payee: PAYEE, asset: ZEC, amount: W(2), firstDue: FIRST_DUE, fundPeriods: 3, reference: REFERENCE, nonce: NONCE, funding };
  assert.equal(hex(m.payCreateIntent({ ...create, every: { months: 1 }, periods: 12 })), v.pay_create);
  assert.equal(hex(m.payCreateIntent({ ...create, every: { months: 1 }, periods: null })), v.pay_create_until_cancelled);
  assert.equal(hex(m.payCreateIntent({ ...create, every: { days: 7 }, periods: 4 })), v.pay_create_days);
  assert.equal(hex(m.payTopUpIntent({ payer: OWNER, schedule, periods: 2, funding })), v.pay_top_up);
  assert.equal(hex(m.payCancelIntent({ payer: OWNER, schedule })), v.pay_cancel);
  assert.equal(hex(m.payOnceIntent({ payer: OWNER, payee: PAYEE, asset: ZEC, amount: W(1), reference: REFERENCE, nonce: NONCE, funding })), v.pay_once);
  assert.equal(hex(m.payRefundIntent({ payee: PAYEE, payment, amount: W(1), funding })), v.pay_refund);
});

test('Zyn Pay: bad inputs throw instead of encoding wrong bytes', () => {
  const schedule = m.scheduleId(OWNER, NONCE);
  const payment = m.payPaymentId(schedule, 0);
  const create = { payer: OWNER, payee: PAYEE, asset: ZEC, amount: W(2), every: { months: 1 }, firstDue: FIRST_DUE, periods: 12, fundPeriods: 3, reference: REFERENCE, nonce: NONCE, funding };
  const bad = (o) => assert.throws(() => m.payCreateIntent({ ...create, ...o }));
  // every
  for (const every of [{}, { days: 1, months: 1 }, { days: 0 }, { months: -1 }, { days: 1.5 }, { months: 2 ** 32 }, { days: NaN }]) bad({ every });
  assert.doesNotThrow(() => m.payCreateIntent({ ...create, every: { days: 0xffffffff } }));
  // u32 fields
  for (const n of [-1, 2 ** 32, 1.5, NaN, '3']) {
    bad({ fundPeriods: n });
    bad({ periods: n });
    assert.throws(() => m.payTopUpIntent({ payer: OWNER, schedule, periods: n, funding }));
    assert.throws(() => m.payPaymentId(schedule, n));
    assert.throws(() => m.refundSource(payment, n));
  }
  // periods: only null means until cancelled
  bad({ periods: undefined });
  assert.doesNotThrow(() => m.payCreateIntent({ ...create, periods: null }));
  // firstDue
  for (const firstDue of [-1, 1.5, 2 ** 53, NaN, undefined, '1', -1n, 2n ** 64n]) bad({ firstDue });
  assert.doesNotThrow(() => m.payCreateIntent({ ...create, firstDue: 0 }));
  assert.equal(hex(m.payCreateIntent({ ...create, firstDue: BigInt(FIRST_DUE) })), hex(m.payCreateIntent(create)));
  assert.doesNotThrow(() => m.payCreateIntent({ ...create, firstDue: 2n ** 64n - 1n }));
  // amounts
  const once = { payer: OWNER, payee: PAYEE, asset: ZEC, amount: W(1), reference: REFERENCE, nonce: NONCE, funding };
  for (const amount of [0n, -1n]) {
    bad({ amount });
    assert.throws(() => m.payOnceIntent({ ...once, amount }));
    assert.throws(() => m.payRefundIntent({ payee: PAYEE, payment, amount, funding }));
  }
  // below the minimum is the caller's rule, not the encoder's
  assert.doesNotThrow(() => m.payOnceIntent({ ...once, amount: 1n }));
});
