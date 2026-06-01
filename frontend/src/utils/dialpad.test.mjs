import test from 'node:test'
import assert from 'node:assert/strict'

import { DIALPAD_KEYS } from './dialpad.mjs'

test('phone dialpad exposes every digit needed for numeric extensions', () => {
  const allDigits = DIALPAD_KEYS.flat().map((k) => k.digit)
  const expected = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '0', '#']

  for (const digit of expected) {
    assert.ok(allDigits.includes(digit), `${digit} should be present on the dialpad`)
  }

  assert.equal(allDigits.length, 12, 'dialpad should have exactly 12 keys')
})

test('dialpad has correct row structure', () => {
  assert.equal(DIALPAD_KEYS.length, 4, 'dialpad should have 4 rows')

  assert.equal(DIALPAD_KEYS[0].length, 3, 'first row should have 3 keys')
  assert.equal(DIALPAD_KEYS[1].length, 3, 'second row should have 3 keys')
  assert.equal(DIALPAD_KEYS[2].length, 3, 'third row should have 3 keys')
  assert.equal(DIALPAD_KEYS[3].length, 3, 'fourth row should have 3 keys')
})

test('dialpad has correct letters for each row', () => {
  assert.equal(DIALPAD_KEYS[0][0].digit, '1')
  assert.equal(DIALPAD_KEYS[0][1].digit, '2')
  assert.equal(DIALPAD_KEYS[0][2].digit, '3')

  assert.equal(DIALPAD_KEYS[1][0].digit, '4')
  assert.equal(DIALPAD_KEYS[1][1].digit, '5')
  assert.equal(DIALPAD_KEYS[1][2].digit, '6')

  assert.equal(DIALPAD_KEYS[2][0].digit, '7')
  assert.equal(DIALPAD_KEYS[2][1].digit, '8')
  assert.equal(DIALPAD_KEYS[2][2].digit, '9')

  assert.equal(DIALPAD_KEYS[3][0].digit, '*')
  assert.equal(DIALPAD_KEYS[3][1].digit, '0')
  assert.equal(DIALPAD_KEYS[3][2].digit, '#')
})

test('dialpad 0 key has + as sublabel for international prefix', () => {
  const zeroKey = DIALPAD_KEYS[3][1]
  assert.equal(zeroKey.digit, '0')
  assert.equal(zeroKey.sub, '+')
})
