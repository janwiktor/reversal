// The engine, from crates/reversal-ios. The app links it and runs the web UI server in-process.

#include <stdbool.h>
#include <stdint.h>

/// Serves `folder` in place of what was served. Returns the port on 127.0.0.1, or 0 if that failed.
uint16_t reversal_engine_start(const char *folder);
void reversal_engine_stop(void);
/// Whether the engine would list a file of this name as a frame.
bool reversal_is_scan(const char *name);
