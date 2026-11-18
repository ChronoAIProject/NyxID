// Fixed, synthetic JPEG vectors. No captured desktop pixels.
export const RED_JPEG = '/9j/4AAQSkZJRgABAgAAAQABAAD//gAQTGF2YzYyLjExLjEwMAD/2wBDAAgEBAQEBAUFBQUFBQYGBgYGBgYGBgYGBgYHBwcICAgHBwcGBgcHCAgICAkJCQgICAgJCQoKCgwMCwsODg4RERT/xABNAAEBAAAAAAAAAAAAAAAAAAAABgEBAQEAAAAAAAAAAAAAAAAAAAYHEAEAAAAAAAAAAAAAAAAAAAAAEQEAAAAAAAAAAAAAAAAAAAAA/8AAEQgABgAIAwESAAISAAMSAP/aAAwDAQACEQMRAD8AixJjfx//2Q==';
export const GREEN_PATCH = '/9j/4AAQSkZJRgABAgAAAQABAAD//gAQTGF2YzYyLjExLjEwMAD/2wBDAAgEBAQEBAUFBQUFBQYGBgYGBgYGBgYGBgYHBwcICAgHBwcGBgcHCAgICAkJCQgICAgJCQoKCgwMCwsODg4RERT/xABMAAEBAAAAAAAAAAAAAAAAAAAABQEBAQAAAAAAAAAAAAAAAAAAAAcQAQAAAAAAAAAAAAAAAAAAAAARAQAAAAAAAAAAAAAAAAAAAAD/wAARCAACAAIDARIAAhIAAxIA/9oADAMBAAIRAxEAPwCsJUImP//Z';
export const BLUE_JPEG = '/9j/4AAQSkZJRgABAgAAAQABAAD//gAQTGF2YzYyLjExLjEwMAD/2wBDAAgEBAQEBAUFBQUFBQYGBgYGBgYGBgYGBgYHBwcICAgHBwcGBgcHCAgICAkJCQgICAgJCQoKCgwMCwsODg4RERT/xABNAAEBAAAAAAAAAAAAAAAAAAAABwEBAQEAAAAAAAAAAAAAAAAAAAUHEAEAAAAAAAAAAAAAAAAAAAAAEQEAAAAAAAAAAAAAAAAAAAAA/8AAEQgAAwAEAwESAAISAAMSAP/aAAwDAQACEQMRAD8Ajg38Sh//2Q==';
export const SESSION = '00112233-4455-6677-8899-aabbccddeeff';
export function vector(jpeg: Uint8Array, sequence = 1n, rectangle?: { width: number; height: number; x: number; y: number; patchWidth: number; patchHeight: number; base: bigint }) {
  const bytes = new Uint8Array(30 + (rectangle ? 24 : 0) + jpeg.length);
  bytes.set([78,89,88,77,1,0]);
  bytes.set(Array.from({length:16}, (_, i) => parseInt(SESSION.replaceAll('-', '').slice(i * 2, i * 2 + 2), 16)), 6);
  const view = new DataView(bytes.buffer); view.setBigUint64(22, sequence);
  if (rectangle) {
    bytes.set([78,89,88,68],30);
    [rectangle.width,rectangle.height,rectangle.x,rectangle.y,rectangle.patchWidth,rectangle.patchHeight].forEach((n,i) => view.setUint16(34+i*2,n));
    view.setBigUint64(46,rectangle.base);
  }
  bytes.set(jpeg, rectangle ? 54 : 30);
  return bytes.buffer;
}
