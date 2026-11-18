import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { Screen } from '../src/terminal/Screen.ts';

test('parses escape sequences across output chunks without rendering string payloads', () => {
  const screen = new Screen(10, 3);
  screen.write('\x1b]52;clipboard');
  screen.write('\x1b\\ready\r\nnext');
  assert.deepEqual(screen.snapshot().lines, ['ready', 'next', '']);
  assert.equal(screen.snapshot().cursor?.row, 1);
});

test('supports character overwrite, cursor restore and display erase', () => {
  const screen = new Screen(10, 3);
  screen.write('abc\x1b7\x1b[1;2HZ\x1b8!');
  assert.equal(screen.snapshot().lines[0], 'aZc!');
  screen.write('\x1b[2J');
  assert.deepEqual(screen.snapshot().lines, ['', '', '']);
});

test('restores the primary buffer after an alternate screen session', () => {
  const screen = new Screen(10, 3);
  screen.write('primary\x1b[?1049h\x1b[?2004hother');
  assert.equal(screen.snapshot().lines[0], 'other');
  assert.equal(screen.bracketedPaste, true);
  screen.resize(8, 4);
  screen.write('\x1b[?2004l\x1b[?1049l');
  assert.equal(screen.snapshot().lines[0], 'primary');
  assert.equal(screen.bracketedPaste, false);
});

test('restores a saved cursor within the resized screen', () => {
  const screen = new Screen(8, 6);
  screen.write('\x1b[6;8H\x1b7');
  screen.resize(5, 2);
  screen.write('\x1b8X');
  assert.equal(screen.snapshot().lines[1], '    X');
});

test('bounds combining characters and resets partial escape state after dropped output', () => {
  const screen = new Screen(10, 3);
  screen.write('a' + '\u0301'.repeat(2_000));
  assert.equal(screen.snapshot().lines[0].length, 32);

  screen.write('\x1b[?2004h\x1b]52;incomplete');
  assert.equal(screen.bracketedPaste, true);
  screen.reset();
  screen.write('visible');
  assert.equal(screen.bracketedPaste, false);
  assert.equal(screen.snapshot().lines[0], 'visible');
});
