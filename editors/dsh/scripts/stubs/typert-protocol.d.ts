export declare class TypertRemoteService { constructor(ctx: any, name: string); }
export declare const Remote: any;
export declare class RemoteError extends Error {
  constructor(code: string, message: string, details: unknown);
}
export interface RemoteErrorDetailsMap {}
