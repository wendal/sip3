import test from 'node:test'
import assert from 'node:assert/strict'

import { callFailureMessage } from './callUiState.mjs'

test('callFailureMessage preserves existing SIP status mapping', () => {
  assert.equal(callFailureMessage({ statusCode: 486 }), '对方忙')
  assert.equal(callFailureMessage({ response: { statusCode: 404 } }), '对方未在线或不存在')
})

test('callFailureMessage keeps audio-only permission and device messages', () => {
  assert.equal(
    callFailureMessage({ name: 'NotFoundError' }),
    '未找到可用麦克风，请连接或启用麦克风后重试',
  )
  assert.equal(
    callFailureMessage({ name: 'NotAllowedError' }),
    '浏览器未授权麦克风，请允许麦克风权限后重试',
  )
})

test('callFailureMessage covers busy media device and generic fallback cases', () => {
  assert.equal(
    callFailureMessage({ name: 'NotReadableError' }, { videoRequested: true }),
    '无法访问麦克风或摄像头，请关闭占用设备的应用后重试',
  )
  assert.equal(
    callFailureMessage({ name: 'NotReadableError' }),
    '无法访问麦克风，请关闭占用设备的应用后重试',
  )
  assert.equal(callFailureMessage({ message: 'custom failure' }), 'custom failure')
})

test('callFailureMessage mentions camera when video devices are missing', () => {
  assert.equal(
    callFailureMessage({ name: 'NotFoundError' }, { videoRequested: true }),
    '未找到可用麦克风或摄像头，请连接或启用设备后重试',
  )
})

test('callFailureMessage mentions camera when video permissions are denied', () => {
  assert.equal(
    callFailureMessage({ name: 'NotAllowedError' }, { videoRequested: true }),
    '浏览器未授权麦克风或摄像头，请允许相关权限后重试',
  )
})
