// Telegram RPC names and the readable errors returned by existing native builds.
// Never display an unrecognized native message: it can include account data.
const reasons: [RegExp, string][] = [
    [/API_ID_PUBLISHED_FLOOD|disabled this API ID/, 'action_errors.disabled'],
    [/API_ID_INVALID|API ID or API Hash is invalid|API Hash cannot be empty/, 'action_errors.credentials'],
    [/PHONE_NUMBER_BANNED|banned this phone number/, 'action_errors.banned'],
    [/PHONE_NUMBER_INVALID|rejected that phone number/, 'action_errors.phone'],
    [/must be registered in an official Telegram app/, 'action_errors.register'],
    [/PHONE_CODE_EXPIRED|authentication code has expired|No (?:active phone login|password session found)/i, 'action_errors.expired'],
    [/PHONE_CODE_(?:INVALID|EMPTY)|authentication code is invalid|Enter the authentication code/, 'action_errors.code'],
    [/PASSWORD_HASH_INVALID|invalid password|password verification failed|two-step verification password is incorrect/i, 'action_errors.password'],
    [/PHONE_(?:PASSWORD|NUMBER)_FLOOD|Too many login/, 'auth.please_wait'],
    [/SMS_CODE_CREATE_FAILED|SEND_CODE_UNAVAILABLE|no additional code-delivery method|could not create a login code/, 'auth.code_delivery_unsupported'],
    [/UPDATE_APP_TO_LOGIN|newer authentication protocol/, 'settings.check_for_updates'],
    [/did not respond|timed out|unreachable|not connected|connection failed/i, 'activity.category_network'],
    [/request is already in progress|replaced by a newer attempt/, 'auth.please_wait'],
];

export function signInErrorKey(message: string): string {
    return reasons.find(([pattern]) => pattern.test(message))?.[1] ?? 'common.operation_failed';
}
