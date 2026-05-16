export function hasActiveVideoMedia(sdp = '') {
  if (!sdp) return false
  const match = sdp.match(/^m=video\s+(\d+)\b/m)
  if (!match) return false
  return Number.parseInt(match[1], 10) > 0
}

export function mediaConstraintsForCallMode(callMode = 'audio') {
  return {
    audio: true,
    video: callMode === 'video',
  }
}

export function resolveNegotiatedCallMode(peerConnection, requestedCallMode = 'audio') {
  if (requestedCallMode !== 'video') {
    return requestedCallMode
  }
  // A rejected remote m=video line (`m=video 0`) means the peer accepted
  // the call as audio-only, so the UI should drop back to audio mode.
  return hasActiveVideoMedia(peerConnection?.remoteDescription?.sdp) ? 'video' : 'audio'
}

export function stopLocalSenderTracks(peerConnection, trackKinds = ['audio', 'video']) {
  peerConnection?.getSenders?.().forEach((sender) => {
    if (!sender.track || !trackKinds.includes(sender.track.kind)) {
      return
    }
    sender.track.enabled = false
    sender.track.stop?.()
  })
}

export function attachCallMedia({
  peerConnection,
  callMode,
  remoteAudio,
  remoteVideo,
  localVideo,
  createStream = () => new MediaStream(),
}) {
  if (!peerConnection) {
    return () => {}
  }

  const remoteStream = createStream()
  const localPreviewStream = callMode === 'video' ? createStream() : null
  const seenRemoteTracks = new Set()
  const seenLocalTracks = new Set()

  function addTrackOnce(stream, seenTracks, track, kinds = []) {
    if (!stream || !track) return
    if (kinds.length > 0 && !kinds.includes(track.kind)) return
    if (seenTracks.has(track)) return
    seenTracks.add(track)
    stream.addTrack(track)
  }

  function bindMediaElements() {
    if (callMode === 'video') {
      if (remoteVideo) remoteVideo.srcObject = remoteStream
      if (remoteAudio) remoteAudio.srcObject = null
    } else {
      if (remoteAudio) remoteAudio.srcObject = remoteStream
      if (remoteVideo) remoteVideo.srcObject = null
    }

    if (localVideo) {
      localVideo.srcObject = localPreviewStream?.getTracks().length ? localPreviewStream : null
    }
  }

  peerConnection.getReceivers().forEach((receiver) => {
    addTrackOnce(remoteStream, seenRemoteTracks, receiver.track)
  })
  peerConnection.getSenders().forEach((sender) => {
    addTrackOnce(localPreviewStream, seenLocalTracks, sender.track, ['video'])
  })

  const handleTrack = (event) => {
    addTrackOnce(remoteStream, seenRemoteTracks, event.track)
    bindMediaElements()
  }

  peerConnection.addEventListener('track', handleTrack)
  bindMediaElements()

  return () => {
    peerConnection.removeEventListener('track', handleTrack)
    if (remoteAudio) remoteAudio.srcObject = null
    if (remoteVideo) remoteVideo.srcObject = null
    if (localVideo) localVideo.srcObject = null
  }
}
