---
title: How To Raise And Handle Object Events
weight: 17
---

The .NET events of objects: a proxy class that declares events and implements an interface's, Rust raising them, and Rust subscribing to any object's events, through a callback or through the runspace's event queue. Engine events, which `New-Event` and `Register-EngineEvent` use, are in [How To Raise Engine Events](How-To-Raise-Engine-Events.md). Source: `crates/pwrs/src/events.rs` (`PsObject::subscribe`, `PsEvents::subscribe`, `take`, `wait`), `crates/pwrs/src/proxy.rs` (`PsThis`, `PsProxy::raise`), `crates/cargo-pwrs/src/bases.rs` (the declarations), `crates/cargo-pwrs/dotnet/Pwrs.Runtime/ObjectEvents.cs` (host-table entries 86 to 89).

## Declaring events

A proxy class declares events of its own with `events`, each a name with its delegate type after a colon, `System.EventHandler` when it gives none:

```rust
#[psclass(
    name = "Hello.Draft",
    mode = proxy,
    implements = ["System.ComponentModel.INotifyPropertyChanged", "System.Windows.Input.ICommand"],
    events = ["Saved", "Ticked: System.EventHandler<Hello.TickArgs>"]
)]
pub struct Draft {
    pub text: String,
    pub saves: i64,
}
```

The class also declares every abstract event of its `base` and of each interface it `implements`, with that event's own delegate type, so `INotifyPropertyChanged.PropertyChanged`, `ICommand.CanExecuteChanged` and `INotifyCollectionChanged.CollectionChanged` need nothing more. An interface's event is public under its name, unless the class already has that name, and is then implemented explicitly and raised as `Interface.Name`, as `INotifyPropertyChanged.PropertyChanged`; a base's is an override. `cargo pwrs build` reads each delegate type from the reference assemblies of both compiles and refuses one that is not a delegate, a name the class or its base already has, and an event an interface or the base already declares for the class. A type argument that names one of the module's own classes, as `Hello.TickArgs` does, is the C# compiler's to check.

Each event keeps its handlers in a field of its own, `__pwrsEvent_` and the event's raise name with every character other than an ASCII letter, digit or `_` written as `_`. The runtime raises the event through that field, so the generated class needs nothing of `Pwrs.Runtime` for its events.

## Raising

A `#[psmethods]` method raises an event of its object through `this: &PsThis`, an argument the method's .NET signature does not have:

```rust
pub fn set_text(&mut self, this: &PsThis, text: String) -> PsResult<()> {
    let was_empty = self.text.is_empty();
    self.text = text;
    let changed = PsType::from_name("System.ComponentModel.PropertyChangedEventArgs").new(&["Text".into_ps()?])?;
    this.raise("PropertyChanged", changed)?;
    if was_empty != self.text.is_empty() {
        this.raise("CanExecuteChanged", ())?;
    }
    Ok(())
}
```

A cmdlet holding the object raises through `PsProxy::raise`. For a delegate `(object sender, T e)`, as `EventHandler`, `EventHandler<T>` and `PropertyChangedEventHandler` are, the object is the sender and Rust gives `e`; `()` gives `EventArgs.Empty` to an `EventHandler`. For any other delegate Rust gives every argument: an array's elements, or one value. Each converts to its parameter's type as PowerShell converts it, at the raise, so a raise that cannot be delivered fails where it is made.

A raise made during a call into the object, from a method, a property read, or a `PsProxy::with` or `with_mut` borrow, is delivered once the call has released the object, in the order raised and on the same thread, before the call returns to its caller. While a `&mut self` method runs, the object refuses every other call on its thread, so a handler run inside it could not read the property it was told about; delivered afterwards, it reads the new value, as a data binding does:

```powershell
$d = [Hello.Draft]::new()
$d.add_PropertyChanged({ param($s, $e) "$($e.PropertyName) is now $($s.Text)" | Write-Host })
$d.SetText('hello')
```

```text
Text is now hello
```

A handler's exception is then the call's own, thrown after the value has changed. When the call itself returned an `Err`, that error is thrown, and the handler's exception goes to standard error. A raise made anywhere else, from a worker thread or a cmdlet not borrowing the object, runs the handlers on its thread before it returns, and a handler's exception is the raise's `Err`.

## Raising from another thread

A clone of `PsThis` is `Send` and `Sync` and keeps the object alive, so a thread the method starts raises later:

```rust
pub fn tick(&self, this: &PsThis, count: i64) -> PsResult<()> {
    let this = this.clone();
    std::thread::spawn(move || {
        for number in 1..=count {
            if let Err(e) = this.raise("Ticked", TickArgs { number }) {
                eprintln!("hello: raising Ticked {number} of {count} failed: {}", e.message);
                return;
            }
        }
    });
    Ok(())
}
```

`Register-ObjectEvent` receives such a raise. Its handler queues each event through the thread pool, so it keeps every event but not the order one thread raised them in. A script block added with `add_Ticked` runs only on the thread of a runspace, so raised from another thread it fails, and the raise returns the engine's error. In pwsh 7.6.9 and Windows PowerShell 5.1:

```text
hello: raising Ticked 1 of 1 failed: PSInvalidOperationException: There is no Runspace available to run scripts in this thread. You can provide one in the DefaultRunspace property of the System.Management.Automation.Runspaces.Runspace type. The script block you attempted to invoke was:  param($s, $e) $null = $e
```

## Subscribing with a callback

`PsObject::subscribe` gives a closure to an event of any object, or to the static event of a type, as C#'s `+=` does. It runs on the raising thread, before the raise goes on, with the sender and the arguments: for a delegate `(object sender, T e)` the sender and `[e]`, for any other no sender and every argument. Its `Err` or its panic is thrown to the raiser. A delegate that returns a value, or takes a `ref`, `out` or `in` argument, is refused.

```rust
let created = watcher.subscribe("Created", move |_sender, args| {
    let name = String::from_ps(&args[0].get("Name")?)?;
    match heard.send((name, std::thread::current().id())) {
        Ok(()) => Ok(()),
        // The command has stopped listening: the file is not counted.
        Err(std::sync::mpsc::SendError(_)) => Ok(()),
    }
})?;
```

Dropping the `PsSubscription` removes the closure and does not wait: a call already running on another thread finishes with its own share of it, and the closure is dropped after the last call returns. `unsubscribe` does the same and returns what went wrong. hello's `Watch-RustFolder` subscribes to a `FileSystemWatcher`, whose events .NET raises on threads of its own, makes three files and writes what the callback heard, and whether it ran off the pipeline thread:

```text
watched-1.txt true
watched-2.txt true
watched-3.txt true
```

## Subscribing through the event queue

`PsEvents::subscribe` queues each raise of an object's event in the runspace's event queue, under a source identifier, where `Get-Event` sees it. The record has the shape `Register-ObjectEvent` gives one, its `Sender` the object and its `SourceArgs` the delegate's arguments, but it is queued on the raising thread, so the records one thread raises keep their order. A phase reads the queue back:

```rust
let events = ps.events()?;
let subscription = events.subscribe(&watcher, "Created", "Hello.WatchedFolder")?;
// ... later, in this or another phase of this command
while let Some(record) = events.wait("Hello.WatchedFolder", None)? {
    ps.write(String::from_ps(&record.source_event_args.get("Name")?)?)?;
}
```

`take` returns every record waiting, oldest first, and removes them, as `Get-Event` and `Remove-Event` do, with one call of each however many wait. `wait` returns the oldest, waiting for one when none is queued, until a raise queues one, the pipeline stops or the timeout passes, `None` for the last two; the queue wakes it as each record arrives and it looks at the pipeline every 50 ms. Both run through the cmdlet whose `events` made the `PsEvents`, on its pipeline thread. A phase reading a steady stream takes in a loop and waits only when nothing is queued:

```rust
let mut records = events.take(id)?;
if records.is_empty() {
    match events.wait(id, None)? {
        Some(record) => records.push(record),
        None => return Ok(()),
    }
}
```

Dropping the `PsEventSubscription` ends it on any thread; the records already queued stay. hello's `Receive-RustTicks` reads 5,000 ticks a worker raised as fast as it could back in the order raised.

## Tests

`examples/hello/tests/ObjectEvents.Tests.ps1` checks the draft's interfaces, `PropertyChanged` read after the setter returns, `CanExecuteChanged`, `Saved` with `EventArgs.Empty`, a handler's exception made the setter's, each worker tick reaching `Register-ObjectEvent`, 5,000 ticks through the queue in order, the script block handler that fails off a runspace's thread, and both folder watchers, in pwsh 7.6.9 and Windows PowerShell 5.1.
