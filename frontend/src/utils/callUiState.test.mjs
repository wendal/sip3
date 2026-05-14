import test from 'node:test'
import assert from 'node:assert/strict'

import { activeCallStates, callFailureMessage } from './callUiState.mjs'

test('outbound dialing state exposes hangup controls', () => {
  assert.deepEqual(activeCallStates, ['incoming', 'dialing', 'active'])
})

test('SIP failure responses are shown as readable phone errors', () => {
  assert.equal(callFailureMessage({ statusCode: 404 }), '对方未在线或不存在')
  assert.equal(callFailureMessage({ statusCode: 486 }), '对方忙')
  assert.equal(callFailureMessage(new Error('transport failed')), 'transport failed')
})

test('Firefox WebRTC media errors explain microphone requirements', () => {
  assert.equal(callFailureMessage({ name: 'NotFoundError', message: 'The object can not be found here.' }), '未找到可用麦克风，请连接或启用麦克风后重试')
  assert.equal(callFailureMessage({ name: 'NotAllowedError' }), '浏览器未授权麦克风，请允许麦克风权限后重试')
})
