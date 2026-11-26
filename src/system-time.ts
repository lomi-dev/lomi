import { useEffect, useState } from "react";
import { api } from "./api";

// The OS 12/24-hour preference, read once per launch (see time_format.rs).
let cached: boolean | null | undefined;
let request: Promise<boolean | null> | undefined;

/** Clock options following the OS setting; empty until it is known. */
export function useSystemClock(): Intl.DateTimeFormatOptions {
  const [hour12, setHour12] = useState(cached);
  useEffect(() => {
    if (cached !== undefined) return;
    let live = true;
    request ??= api<boolean | null>("system_hour12")
      .catch(() => null)
      .then((value) => (cached = value));
    void request.then((value) => live && setHour12(value));
    return () => {
      live = false;
    };
  }, []);
  // hourCycle avoids the "24:05" that hour12: false gives in some engines.
  return hour12 == null ? {} : { hourCycle: hour12 ? "h12" : "h23" };
}
