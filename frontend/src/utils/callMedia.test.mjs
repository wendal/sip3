import test from 'node:test'
import assert from 'node:assert/strict'

import {
  attachCallMedia,
  hasActiveVideoMedia,
  mediaConstraintsForCallMode,
  resolveNegotiatedCallMode,
  stopLocalSenderTracks,
} from './callMedia.mjs'

class FakeStream {
  constructor() {
    this.tracks = []
  }

  addTrack(track) {
    if (!this.tracks.includes(track)) {
      this.tracks.push(track)
    }
  }

  getTracks() {
    return [...this.tracks]
  }
}

class FakeElement {
  constructor() {
    this.srcObject = null
  }
}

class FakePeerConnection {
  constructor({ receivers = [], senders = [] } = {}) {
    this.receivers = receivers.map((track) => ({ track }))
    this.senders = senders.map((track) => ({ track }))
    this.trackListeners = new Set()
  }

  getReceivers() {
    return this.receivers
  }

  getSenders() {
    return this.senders
  }

  addEventListener(name, listener) {
    if (name === 'track') {
      this.trackListeners.add(listener)
    }
  }

  removeEventListener(name, listener) {
    if (name === 'track') {
      this.trackListeners.delete(listener)
    }
  }

  emitTrack(track) {
    for (const listener of this.trackListeners) {
      listener({ track })
    }
  }
}

test('hasActiveVideoMedia detects active video m-lines', () => {
  assert.equal(hasActiveVideoMedia(null), false)
  assert.equal(hasActiveVideoMedia('m=audio 49170 RTP/AVP 0\r\n'), false)
  assert.equal(hasActiveVideoMedia('m=video 51372 RTP/AVP 96\r\n'), true)
  assert.equal(hasActiveVideoMedia('m=video 0 RTP/AVP 96\r\n'), false)
})

test('mediaConstraintsForCallMode maps audio and video modes', () => {
  assert.deepEqual(mediaConstraintsForCallMode('audio'), { audio: true, video: false })
  assert.deepEqual(mediaConstraintsForCallMode('video'), { audio: true, video: true })
})

test('resolveNegotiatedCallMode falls back to audio when remote answer rejects video', () => {
  assert.equal(
    resolveNegotiatedCallMode({ remoteDescription: { sdp: 'm=audio 49170 RTP/AVP 0\r\nm=video 0 RTP/AVP 96\r\n' } }, 'video'),
    'audio',
  )
  assert.equal(
    resolveNegotiatedCallMode({ remoteDescription: { sdp: 'm=audio 49170 RTP/AVP 0\r\nm=video 51372 RTP/AVP 96\r\n' } }, 'video'),
    'video',
  )
  assert.equal(resolveNegotiatedCallMode(null, 'audio'), 'audio')
})

test('attachCallMedia binds audio sessions to the remote audio element', () => {
  const remoteAudio = new FakeElement()
  const remoteVideo = new FakeElement()
  const localVideo = new FakeElement()
  const remoteTrack = { kind: 'audio', id: 'remote-audio' }
  const peerConnection = new FakePeerConnection({ receivers: [remoteTrack] })

  const cleanup = attachCallMedia({
    peerConnection,
    callMode: 'audio',
    remoteAudio,
    remoteVideo,
    localVideo,
    createStream: () => new FakeStream(),
  })

  assert.deepEqual(remoteAudio.srcObject.getTracks(), [remoteTrack])
  assert.equal(remoteVideo.srcObject, null)
  assert.equal(localVideo.srcObject, null)

  cleanup()
  assert.equal(remoteAudio.srcObject, null)
})

test('attachCallMedia binds video sessions to remote and local video elements', () => {
  const remoteAudio = new FakeElement()
  const remoteVideo = new FakeElement()
  const localVideo = new FakeElement()
  const remoteAudioTrack = { kind: 'audio', id: 'remote-audio' }
  const remoteVideoTrack = { kind: 'video', id: 'remote-video' }
  const localCameraTrack = { kind: 'video', id: 'local-video' }
  const peerConnection = new FakePeerConnection({
    receivers: [remoteAudioTrack],
    senders: [localCameraTrack],
  })

  attachCallMedia({
    peerConnection,
    callMode: 'video',
    remoteAudio,
    remoteVideo,
    localVideo,
    createStream: () => new FakeStream(),
  })
  peerConnection.emitTrack(remoteVideoTrack)

  assert.equal(remoteAudio.srcObject, null)
  assert.deepEqual(remoteVideo.srcObject.getTracks(), [remoteAudioTrack, remoteVideoTrack])
  assert.deepEqual(localVideo.srcObject.getTracks(), [localCameraTrack])
})

test('stopLocalSenderTracks can stop only local video senders', () => {
  const audioTrack = { kind: 'audio', stopped: false, stop() { this.stopped = true } }
  const videoTrack = { kind: 'video', stopped: false, stop() { this.stopped = true } }
  const peerConnection = new FakePeerConnection({ senders: [audioTrack, videoTrack] })

  stopLocalSenderTracks(peerConnection, ['video'])

  assert.equal(audioTrack.stopped, false)
  assert.equal(videoTrack.stopped, true)
})

test('stopLocalSenderTracks stops all local capture tracks by default', () => {
  const audioTrack = { kind: 'audio', stopped: false, stop() { this.stopped = true } }
  const videoTrack = { kind: 'video', stopped: false, stop() { this.stopped = true } }
  const peerConnection = new FakePeerConnection({ senders: [audioTrack, videoTrack] })

  stopLocalSenderTracks(peerConnection)

  assert.equal(audioTrack.stopped, true)
  assert.equal(videoTrack.stopped, true)
})
