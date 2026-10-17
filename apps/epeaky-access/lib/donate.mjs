// Where donations go: the team's own addresses, one per chain. Checked on 17
// Oct 2026: ZEC is the team account's deposit address on the vault (asked of
// the node), SOL is vault 1 of the team's Squads multisig (3 of 4; derived from
// its seeds, apart from the bridge's vault 0), ETH is the team's Safe (2 of 3,
// takes plain ETH). The page, its Copy buttons and its QR codes all come from
// here; tests/donate.test.mjs holds them to it.
export const DONATE = [
  { id: 'zec', name: 'Zcash', unit: 'ZEC', min: '0.01', scheme: 'zcash', address: 'u1ry6fjswf53yaz0lqv93mhe9nu5fja39amw92tec60slla49lj3f2d8ft9k62q9cly3fkpmu88zpwqy97uyznru74u0deddr53vsquesh' },
  { id: 'sol', name: 'Solana', unit: 'SOL', min: '0.1', scheme: 'solana', address: 'FY2DmpewnPEj7eXhuRMapA9wJ83KkCfaxvSPLYXuPftL' },
  { id: 'eth', name: 'Ethereum', unit: 'ETH', min: '0.01', scheme: 'ethereum', address: '0xEE741530abB9eaD1C7d6E9C58E5Df0D43F39b983' },
];

/// What each QR code holds: the address with its chain's URI scheme
/// (ZIP-321, Solana Pay, EIP-681), so a wallet that scans it picks the chain.
export const uri = (d) => `${d.scheme}:${d.address}`;
