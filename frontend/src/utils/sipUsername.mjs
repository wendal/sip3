export const SIP_USERNAME_RULE_MESSAGE = 'SIP 用户名必须是 3-6 位数字分机号'

export function isValidSipUsername(username) {
  return /^[0-9]{3,6}$/.test(username)
}
