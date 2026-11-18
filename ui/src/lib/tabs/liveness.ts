// Whether a tab has a connected session, derived from the events of its session. Sessions
// still connecting or waiting for a credential, certificate or host key do not count.

import type { ConnectionKind } from "../tree/model";
import type { SessionEvent } from "./session";

/** The connected state of a session of `kind` after `event`, given the state before it. */
export function liveAfter(kind: ConnectionKind, live: boolean, event: SessionEvent): boolean {
  switch (kind) {
    case "web":
      return false;
    case "ssh":
    case "localShell":
      if (event.type === "data") return true;
      if (event.type === "exit" || event.type === "error") return false;
      return live;
    case "rdp":
    case "vnc":
      if (event.type === "resize" || event.type === "rect") return true;
      if (event.type === "closed" || event.type === "error") return false;
      return live;
  }
}
