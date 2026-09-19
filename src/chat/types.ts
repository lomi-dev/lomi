import type { UIMessage } from "ai";
import type { Provider } from "./provider-presets";
export interface Origin {
  projectId: string;
  projectName: string;
  workspaceId: string;
  workspaceName: string;
}
export interface Config {
  connectionId: string | null;
  model: string;
  system: string;
  maxOutputTokens: number;
  temperature: number | null;
  configured: boolean;
}
export interface Conversation {
  id: string;
  title: string;
  origin: Origin;
  config: Config;
  revision: number;
  activeLeafId: string | null;
  updatedAt: number;
  pinned: boolean;
}
export interface SavedMessage extends UIMessage {
  parentId: string | null;
  partsVersion: number;
  status: string;
  previousVariant: string | null;
  nextVariant: string | null;
  attachments: string[];
}
export interface Draft {
  text: string;
  revision: number;
  attachments: string[];
}
export interface Loaded {
  conversation: Conversation;
  draft: Draft;
  messages: SavedMessage[];
  hasOlder: boolean;
  request: { id: string; assistantId: string; status: string } | null;
}
export interface Start {
  requestId: string;
  conversationId: string;
  assistantId: string;
  userId: string;
  expectedRevision: number;
  draftRevision: number;
  action: "send" | "edit" | "retry";
  targetId: string | null;
  text: string;
}
export interface Accepted {
  requestId: string;
  assistantId: string;
  userId: string;
  draftRevision: number;
  repeated: boolean;
}
export interface Connection {
  id: string;
  name: string;
  provider: Provider;
  enabled: boolean;
  credentialRevision: number;
  secretMode: "system" | "session";
  secretId: string | null;
  models: string[];
  testedModel: string | null;
  testStatus: string | null;
}
export interface Preferences {
  version: number;
  revision: number;
  connections: Connection[];
  defaults: Config;
  sendMode: "enter" | "modifier-enter";
}
export interface Attachment {
  id: string;
  name: string;
  mime: string;
  size: number;
}
export const defaultConfig: Config = {
  connectionId: null,
  model: "",
  system: "",
  maxOutputTokens: 4096,
  temperature: null,
  configured: false,
};
export const textOf = (message: UIMessage) =>
  message.parts
    .filter((p) => p.type === "text")
    .map((p) => p.text)
    .join("\n");
