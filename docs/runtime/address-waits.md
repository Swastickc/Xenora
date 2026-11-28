# Guest address waits

Hosted PE programs can import `WaitOnAddress`, `WakeByAddressSingle` and
`WakeByAddressAll` from `api-ms-win-core-synch-l1-2-0.dll`. These services
coordinate threads in one guest process through the cooperative scheduler.

`WaitOnAddress` reads both guest ranges using the requested width of 1, 2,
4 or 8 bytes. Different values return success immediately. An equal value
with a zero timeout returns false with `ERROR_TIMEOUT`. With a nonzero
timeout or `INFINITE`, the caller yields to other guest threads and waits
on the original address. A finite deadline uses the runtime clock. An
invalid width returns false with `ERROR_INVALID_PARAMETER`. An unreadable
range produces a checked guest memory fault. If the scheduler is detached,
a call that would block returns false with `ERROR_NOT_SUPPORTED`.

`WakeByAddressSingle` selects the oldest eligible wait at the supplied
address. `WakeByAddressAll` selects every eligible wait at that address.
An expired or suspended wait does not consume a wake. A wake recorded before
the deadline remains valid until the thread resumes. Waking an address has no
effect on future waiters, and a selected thread can return even if the bytes
still match. A guest should recheck its condition after returning. Thread
exit releases its pending wait.

The public behavior is described in the Microsoft references for
[`WaitOnAddress`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitonaddress),
[`WakeByAddressSingle`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-wakebyaddresssingle)
and [`WakeByAddressAll`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-wakebyaddressall).

On Apple Silicon, `cargo test --locked -p xenora-loader --test guest_address_wait`
builds the original C guest with native Clang and LLD, then runs it through
the ARM64 translator with the mock clock at several scheduler quanta. The
guest checks FIFO wake selection, address isolation, suspension, timeouts
and successful worker joins.
