// 手机网页里没有 Tauri。见 build.mjs
export const convertFileSrc = (_path: string) => "";
// 本地规则（lib/parse.ts）连着 lib/api.ts 一起打包进来，手机上只用 parseLocal，不会真的调用
export const invoke = (cmd: string): Promise<never> => Promise.reject(new Error("手机上没有 " + cmd));
