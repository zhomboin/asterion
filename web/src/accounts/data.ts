export type Account = {
  username: string;
  uid: number;
  gid: number;
  shell: string;
  home: string;
  groups: string[];
  system: boolean;
};

export type Accounts = { users: Account[]; truncated: boolean };
export type Keys = { keys: { type: string; fingerprint: string; comment: string }[]; truncated: boolean };
