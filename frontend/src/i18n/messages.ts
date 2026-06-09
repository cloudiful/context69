import { en } from "./messages/en";
import { zhCN } from "./messages/zh-CN";

export const messages = {
  en,
  "zh-CN": zhCN,
};

export type MessageSchema = typeof messages.en;
