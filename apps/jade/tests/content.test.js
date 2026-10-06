// The content store's picture check: dimensions read from the bytes, and a
// picture whose bytes are not the type it names is refused (audit, 6 Oct 2026).
import test from 'node:test';
import assert from 'node:assert/strict';

const { imageSize } = await import('../functions/api/content/[[path]].js');
const url = (type, bytes) => `data:image/${type};base64,${Buffer.from(bytes).toString('base64')}`;

test('sizes come from the picture itself', () => {
  const png = Buffer.alloc(33); png.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]); png.writeUInt32BE(512, 16); png.writeUInt32BE(300, 20);
  assert.deepEqual(imageSize(url('png', png)), { w: 512, h: 300 });
  const gif = Buffer.alloc(13); gif.write('GIF89a'); gif.writeUInt16LE(5000, 6); gif.writeUInt16LE(10, 8);
  assert.deepEqual(imageSize(url('gif', gif)), { w: 5000, h: 10 }, 'a huge one is seen as huge');
  const jpg = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0x00, 0x02, 0x00, 0x03, 0, 0, 0, 0]);
  assert.deepEqual(imageSize(url('jpeg', jpg)), { h: 256, w: 512 });
  assert.equal(imageSize(url('png', jpg)), null, 'JPEG bytes named PNG');
});
