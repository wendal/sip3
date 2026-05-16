export const activeCallStates = ['incoming', 'dialing', 'active']

export function callFailureMessage(error, { videoRequested = false } = {}) {
  const statusCode = error?.statusCode || error?.response?.statusCode
  if (statusCode === 404) return '对方未在线或不存在'
  if (statusCode === 486) return '对方忙'
  if (statusCode === 403) return '当前账号无权呼叫'
  if (statusCode) return `呼叫失败：SIP ${statusCode}`
  if (error?.name === 'NotFoundError' || error?.message === 'The object can not be found here.') {
    return videoRequested
      ? '未找到可用麦克风或摄像头，请连接或启用设备后重试'
      : '未找到可用麦克风，请连接或启用麦克风后重试'
  }
  if (error?.name === 'NotAllowedError' || error?.name === 'SecurityError') {
    return videoRequested
      ? '浏览器未授权麦克风或摄像头，请允许相关权限后重试'
      : '浏览器未授权麦克风，请允许麦克风权限后重试'
  }
  if (error?.name === 'NotReadableError') {
    return videoRequested
      ? '无法访问麦克风或摄像头，请关闭占用设备的应用后重试'
      : '无法访问麦克风，请关闭占用设备的应用后重试'
  }
  return error?.message || '呼叫失败'
}
