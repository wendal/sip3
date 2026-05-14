import test from 'node:test'
import assert from 'node:assert/strict'

import { isValidSipUsername, SIP_USERNAME_RULE_MESSAGE } from './sipUsername.mjs'

test('accepts only 3-6 digit SIP extension usernames', () => {
  for (const username of ['100', '1001', '999999']) {
    assert.equal(isValidSipUsername(username), true, `${username} should be accepted`)
  }

  for (const username of ['', '12', '1000000', 'alice', '100a', '10 01']) {
    assert.equal(isValidSipUsername(username), false, `${username} should be rejected`)
  }
})

test('explains the SIP username rule clearly', () => {
  assert.equal(SIP_USERNAME_RULE_MESSAGE, 'SIP 用户名必须是 3-6 位数字分机号')
})
