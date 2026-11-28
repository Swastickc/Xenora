# USER32 thread messages

Headless PE applications can post messages to another live thread in the
same guest process. A thread establishes its queue with `PeekMessageA` or
`PeekMessageW`, including an empty peek. After that call,
`PostThreadMessageA` and `PostThreadMessageW` can enqueue a message for its
guest thread ID without waiting for delivery. Posting fails when there is
no queue for the ID or when the receiver has terminated. Each queue holds
up to 10,000 posted messages.

The 48-byte Win64 `MSG` contains a null window handle, the message ID, the
full pointer-sized `wParam` and `lParam`, and the guest clock's milliseconds
at posting. The cursor coordinates and private field are zero. In the
windowless model, both a null window selector and `HWND(-1)` select these
thread messages. Two zero filter bounds accept any message. A peek takes
the oldest matching record, while `PM_REMOVE` consumes it after a successful
copy. `PM_NOREMOVE` leaves it available. Empty peeks leave the output buffer
intact, and an invalid destination leaves a queued message intact. A posted
`WM_QUIT` matches regardless of the message range. Either removal mode can
also use `PM_NOYIELD`. Category flags remain outside this subset.

Applications can synchronize the producer and consumer with the existing
guest event services. Signal a ready event after the receiver's first
peek, post messages on the producer, then signal a delivery event before
polling on the receiver. Threads can join and release their handles
through the normal cooperative scheduler.

This subset exposes polling for posted thread messages. The existing
message wait service handles its object side. Windows, sent window
messages and hardware input remain outside this guest queue.

See Microsoft's [PostThreadMessage](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postthreadmessagew),
[PeekMessage](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-peekmessagew)
and [MSG](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-msg)
contracts.
