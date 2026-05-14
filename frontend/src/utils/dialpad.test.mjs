import test from 'node:test'
import assert from 'node:assert/strict'

import { DIALPAD_KEYS } from './dialpad.mjs'

test('phone dialpad exposes every digit needed for numeric extensions', () => {
  const digits = DIALPAD_KEYS.flat().map((key) => key.digit)

  assert.deepEqual(digits, ['1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '0', '#'])
})
