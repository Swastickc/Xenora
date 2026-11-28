# OEM environment blocks

The guest can enumerate its process environment through the narrow
`kernel32.dll!GetEnvironmentStrings` and `GetEnvironmentStringsA` exports.
The documented process-environment API set uses the unsuffixed name.
`FreeEnvironmentStringsA` releases the address returned by either narrow
entry point.

Each call takes a fresh snapshot of the guest environment in
case-insensitive name order. The bytes are `NAME=VALUE` strings separated
by NUL, with two trailing NUL bytes even for an empty environment. Narrow
snapshots use the runtime's OEM page, CP437. A character that the page
cannot represent is replaced by `?`.

The caller can hold multiple snapshots. Updates through the guest's
environment APIs affect later snapshots. The narrow free operation checks
an exact live base from the narrow allocator, so an unrelated heap
allocation or a wide block cannot be released through it. Keep the matching
`GetEnvironmentStringsW` and `FreeEnvironmentStringsW` calls for UTF-16.

See Microsoft's [GetEnvironmentStrings](https://learn.microsoft.com/en-us/windows/win32/api/processenv/nf-processenv-getenvironmentstrings)
and [FreeEnvironmentStringsA](https://learn.microsoft.com/en-us/windows/win32/api/processenv/nf-processenv-freeenvironmentstringsa)
contracts.
