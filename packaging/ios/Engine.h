// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The engine, from crates/reversal-ios. The app links it and runs the web UI server in-process.

#include <stdbool.h>
#include <stdint.h>

/// Serves `folder` in place of what was served. Returns the port on 127.0.0.1, or 0 if that failed.
uint16_t reversal_engine_start(const char *folder);
void reversal_engine_stop(void);
/// Whether the engine would list a file of this name as a frame.
bool reversal_is_scan(const char *name);
