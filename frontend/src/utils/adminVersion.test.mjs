import test from 'node:test'
import assert from 'node:assert/strict'

import { adminVersionText } from './adminVersion.mjs'

test('adminVersionText renders frontend version footer text', () => {
  assert.equal(adminVersionText('1.3.0'), 'v1.3.0 · Open Source SIP Server')
})
