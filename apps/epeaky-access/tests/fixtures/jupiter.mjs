// A buy as this page makes it, for tests: the recorded buy's balances (a real
// 0.19 SOL buy of $EPEAKY), with its instructions in the shape the page's
// Jupiter swap has (decoded from /api/swap, 19 Oct 2026): compute budget,
// the payer's wrapped-SOL account, the SOL into it, the Jupiter route, the
// account closed back to the payer.
export const P = {
  budget: 'ComputeBudget111111111111111111111111111111',
  ata: 'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL',
  system: '11111111111111111111111111111111',
  token: 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA',
  jupiter: 'JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4',
};
export const WSOL = 'So11111111111111111111111111111111111111112';
export const WRAPPED = 'GYqEjLCPsXNDKMgHH9Z7g9BwNiL1ovdqNSpdCFawgFHM';

export function jupiterize(t, lamports = 190_000_000) {
  const payer = t.transaction.message.accountKeys[0].pubkey;
  t.transaction.message.instructions = [
    { programId: P.budget, accounts: [], data: '3DTZbgwsozUF' },
    { programId: P.budget, accounts: [], data: 'Fj2Eoy' },
    { program: 'spl-associated-token-account', programId: P.ata, parsed: { type: 'createIdempotent', info: { account: WRAPPED, mint: WSOL, source: payer, wallet: payer, systemProgram: P.system, tokenProgram: P.token } } },
    { program: 'system', programId: P.system, parsed: { type: 'transfer', info: { source: payer, destination: WRAPPED, lamports } } },
    { program: 'spl-token', programId: P.token, parsed: { type: 'syncNative', info: { account: WRAPPED } } },
    { programId: P.jupiter, accounts: [payer, WRAPPED], data: 'route' },
    { program: 'spl-token', programId: P.token, parsed: { type: 'closeAccount', info: { account: WRAPPED, destination: payer, owner: payer } } },
  ];
  return t;
}
