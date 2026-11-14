import test from "node:test";
import assert from "node:assert/strict";
import { imageSize } from "../src/deck/image.mjs";

const uri = (mime, bytes) =>
  `data:image/${mime};base64,${Buffer.from(bytes).toString("base64")}`;
const svg = (text) => uri("svg+xml", text);

const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const PNG_4_BY_3 =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEElEQVR4nGOQs4qHIwacHACkYQiVfa22owAAAABJRU5ErkJggg==";
// A 5 × 3 baseline JPEG (macOS sips): JFIF, Exif and Photoshop segments
// come before its SOF0 frame header.
const JPEG_5_BY_3 =
  "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAASABIAAD/4QBMRXhpZgAATU0AKgAAAAgAAYdpAAQAAAABAAAAGgAAAAAAA6ABAAMAAAABAAEAAKACAAQAAAABAAAABaADAAQAAAABAAAAAwAAAAD/7QA4UGhvdG9zaG9wIDMuMAA4QklNBAQAAAAAAAA4QklNBCUAAAAAABDUHYzZjwCyBOmACZjs+EJ+/8AAEQgAAwAFAwEiAAIRAQMRAf/EAB8AAAEFAQEBAQEBAAAAAAAAAAABAgMEBQYHCAkKC//EALUQAAIBAwMCBAMFBQQEAAABfQECAwAEEQUSITFBBhNRYQcicRQygZGhCCNCscEVUtHwJDNicoIJChYXGBkaJSYnKCkqNDU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6g4SFhoeIiYqSk5SVlpeYmZqio6Slpqeoqaqys7S1tre4ubrCw8TFxsfIycrS09TV1tfY2drh4uPk5ebn6Onq8fLz9PX29/j5+v/EAB8BAAMBAQEBAQEBAQEAAAAAAAABAgMEBQYHCAkKC//EALURAAIBAgQEAwQHBQQEAAECdwABAgMRBAUhMQYSQVEHYXETIjKBCBRCkaGxwQkjM1LwFWJy0QoWJDThJfEXGBkaJicoKSo1Njc4OTpDREVGR0hJSlNUVVZXWFlaY2RlZmdoaWpzdHV2d3h5eoKDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uLj5OXm5+jp6vLz9PX29/j5+v/bAEMAAgICAgICAwICAwUDAwMFBgUFBQUGCAYGBgYGCAoICAgICAgKCgoKCgoKCgwMDAwMDA4ODg4ODw8PDw8PDw8PD//bAEMBAgICBAQEBwQEBxALCQsQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEP/dAAQAAf/aAAwDAQACEQMRAD8A+rKKKK/ic/eD/9k=";

test("imageSize reads a PNG's IHDR", () => {
  assert.deepEqual(imageSize(PIXEL), { width: 1, height: 1 });
  assert.deepEqual(imageSize(PNG_4_BY_3), { width: 4, height: 3 });
});

test("imageSize walks a JPEG's markers to its frame header", () => {
  assert.deepEqual(imageSize(JPEG_5_BY_3), { width: 5, height: 3 });
  // A progressive (SOF2) frame of 640 × 480 after a DQT segment and fill bytes.
  const progressive = [
    0xff, 0xd8, 0xff, 0xdb, 0x00, 0x04, 0x00, 0x00, 0xff, 0xff, 0xc2, 0x00,
    0x11, 0x08, 0x01, 0xe0, 0x02, 0x80, 0x03,
  ];
  assert.deepEqual(imageSize(uri("jpeg", progressive)), {
    width: 640,
    height: 480,
  });
  // An extended sequential (SOF1) frame of 300 × 200 right after SOI.
  const extended = [
    0xff, 0xd8, 0xff, 0xc1, 0x00, 0x11, 0x08, 0x00, 0xc8, 0x01, 0x2c, 0x03,
  ];
  assert.deepEqual(imageSize(uri("jpeg", extended)), {
    width: 300,
    height: 200,
  });
});

test("imageSize reads an SVG's width and height, else its viewBox", () => {
  const ns = 'xmlns="http://www.w3.org/2000/svg"';
  assert.deepEqual(imageSize(svg(`<svg ${ns} viewBox="0 0 400 300"/>`)), {
    width: 400,
    height: 300,
  });
  assert.deepEqual(
    imageSize(
      svg(
        `<?xml version="1.0"?><svg stroke-width="2" width="120px" height='80' ${ns}></svg>`,
      ),
    ),
    { width: 120, height: 80 },
  );
  // Sizes in other units or percentages fall back to the viewBox.
  assert.deepEqual(
    imageSize(
      svg(`<svg width="100%" height="10cm" viewBox="0,0,16,9" ${ns}/>`),
    ),
    { width: 16, height: 9 },
  );
});

test("imageSize returns null for what it cannot read", () => {
  for (const value of [
    "data:image/png;base64,AAAA",
    "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
    uri("jpeg", [0xff, 0xd8, 0xff, 0xda, 0x00, 0x02]),
    uri("jpeg", [0xff, 0xd8, 0x12, 0x34]),
    svg('<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>'),
    svg('<svg width="0" height="10"/>'),
    "attachment:abc",
    "not a picture",
    undefined,
  ])
    assert.equal(imageSize(value), null, String(value).slice(0, 40));
});
