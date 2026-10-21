import sys

ROOMS = ("Friends, 3 unread messages", "2 mentions General", "Unread messages, News", "Quiet room")


def gtk_main(chat):
    import gi
    gi.require_version("Gtk", "3.0")
    from gi.repository import GLib, Gtk
    win = Gtk.Window(title="Lighting Test Chat" if chat else "Lighting Test App")
    win.connect("destroy", Gtk.main_quit)
    grid = Gtk.Grid(row_spacing=8, column_spacing=8, margin=12)
    win.add(grid)
    if not chat:
        win.set_default_size(460, 220)
        label = Gtk.Label(label="Name:")
        entry = Gtk.Entry()
        label.set_mnemonic_widget(entry)
        entry.get_accessible().set_name("Name:")
        go = Gtk.Button(label="Go")
        box = Gtk.CheckButton(label="Remember me")
        status = Gtk.Label(label="Status: ready", xalign=0)
        state = {"n": 0}

        def clicked(_):
            state["n"] += 1
            status.set_text("Status: clicked %d with %s" % (state["n"], entry.get_text() or "nothing"))

        go.connect("clicked", clicked)
        grid.attach(label, 0, 0, 1, 1)
        grid.attach(entry, 1, 0, 2, 1)
        grid.attach(go, 1, 1, 1, 1)
        grid.attach(box, 2, 1, 1, 1)
        grid.attach(status, 0, 2, 3, 1)
    else:
        win.set_default_size(640, 360)
        msgs = Gtk.ListBox()
        msgs.get_accessible().set_name("Nachrichten in Test")
        rooms = Gtk.ListBox()
        rooms.get_accessible().set_name("Kanaele")
        entry = Gtk.Entry()
        entry.get_accessible().set_name("Nachricht an Test:")

        def add(lb, text):
            row = Gtk.ListBoxRow()
            row.add(Gtk.Label(label=text, xalign=0))
            row.get_accessible().set_name(text)
            lb.add(row)
            row.show_all()

        def send(_):
            text = entry.get_text()
            if text.strip():
                add(msgs, "me: " + text)
                entry.set_text("")
                GLib.timeout_add(300, lambda: add(msgs, "peer: ok, %d chars" % len(text)) and False)

        entry.connect("activate", send)
        add(msgs, "peer: hi, welcome")
        for r in ROOMS:
            add(rooms, r)
        grid.attach(Gtk.Label(label="Nachrichten in Test", xalign=0), 0, 0, 1, 1)
        grid.attach(msgs, 0, 1, 1, 1)
        grid.attach(Gtk.Label(label="Nachricht an Test:", xalign=0), 0, 2, 1, 1)
        grid.attach(entry, 0, 3, 1, 1)
        grid.attach(Gtk.Label(label="Kanaele", xalign=0), 1, 0, 1, 1)
        grid.attach(rooms, 1, 1, 1, 1)
        msgs.set_size_request(370, 200)
        rooms.set_size_request(200, 200)
    win.move(200, 200)
    win.show_all()
    Gtk.main()


def mac_main(chat):
    import objc
    from AppKit import (NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSBackingStoreBuffered, NSButton,
                        NSMakeRect, NSObject, NSScrollView, NSSwitchButton, NSTableColumn, NSTableView, NSTextField,
                        NSWindow)
    from Foundation import NSTimer

    app = NSApplication.sharedApplication()
    app.setActivationPolicy_(NSApplicationActivationPolicyRegular)
    style = 1 | 2 | 4 | 8
    title = "Lighting Test Chat" if chat else "Lighting Test App"
    win = NSWindow.alloc().initWithContentRect_styleMask_backing_defer_(NSMakeRect(200, 300, 640 if chat else 460, 360 if chat else 220),
                                                                         style, NSBackingStoreBuffered, False)
    win.setTitle_(title)
    view = win.contentView()

    def label(text, x, y, w, h):
        t = NSTextField.labelWithString_(text)
        t.setFrame_(NSMakeRect(x, y, w, h))
        view.addSubview_(t)
        return t

    class Rows(NSObject):
        def init(self):
            self = objc.super(Rows, self).init()
            self.items = []
            return self

        def numberOfRowsInTableView_(self, tv):
            return len(self.items)

        def tableView_objectValueForTableColumn_row_(self, tv, col, row):
            return self.items[row]

    keep = []
    if not chat:
        label("Name:", 20, 170, 60, 22)
        entry = NSTextField.alloc().initWithFrame_(NSMakeRect(90, 168, 320, 26))
        entry.setAccessibilityLabel_("Name:")
        view.addSubview_(entry)
        status = label("Status: ready", 20, 40, 400, 24)
        state = {"n": 0}

        class Target(NSObject):
            def go_(self, sender):
                state["n"] += 1
                status.setStringValue_("Status: clicked %d with %s" % (state["n"], entry.stringValue() or "nothing"))

        target = Target.alloc().init()
        keep.append(target)
        go = NSButton.buttonWithTitle_target_action_("Go", target, "go:")
        go.setFrame_(NSMakeRect(90, 110, 100, 32))
        view.addSubview_(go)
        box = NSButton.alloc().initWithFrame_(NSMakeRect(210, 110, 200, 32))
        box.setButtonType_(NSSwitchButton)
        box.setTitle_("Remember me")
        view.addSubview_(box)
    else:
        def table(name, x, y, w, h, items):
            src = Rows.alloc().init()
            src.items = list(items)
            tv = NSTableView.alloc().initWithFrame_(NSMakeRect(0, 0, w, h))
            col = NSTableColumn.alloc().initWithIdentifier_("c")
            col.setWidth_(w - 4)
            tv.addTableColumn_(col)
            tv.setHeaderView_(None)
            tv.setDataSource_(src)
            tv.setAccessibilityLabel_(name)
            sv = NSScrollView.alloc().initWithFrame_(NSMakeRect(x, y, w, h))
            sv.setDocumentView_(tv)
            view.addSubview_(sv)
            keep.extend([src, tv])
            return src, tv

        label("Nachrichten in Test", 16, 330, 300, 20)
        msgs, mtv = table("Nachrichten in Test", 16, 110, 370, 210, ["peer: hi, welcome"])
        label("Kanaele", 400, 330, 200, 20)
        table("Kanaele", 400, 110, 200, 210, ROOMS)
        label("Nachricht an Test:", 16, 80, 300, 20)
        entry = NSTextField.alloc().initWithFrame_(NSMakeRect(16, 50, 370, 26))
        entry.setAccessibilityLabel_("Nachricht an Test:")
        view.addSubview_(entry)

        class Sender(NSObject):
            def send_(self, sender):
                text = entry.stringValue()
                if not text.strip():
                    return
                msgs.items.append("me: " + text)
                mtv.reloadData()
                entry.setStringValue_("")
                self.n = len(text)
                NSTimer.scheduledTimerWithTimeInterval_target_selector_userInfo_repeats_(0.3, self, "answer:", None, False)

            def answer_(self, timer):
                msgs.items.append("peer: ok, %d chars" % self.n)
                mtv.reloadData()

        sender = Sender.alloc().init()
        keep.append(sender)
        entry.setTarget_(sender)
        entry.setAction_("send:")

    class Delegate(NSObject):
        def applicationShouldTerminateAfterLastWindowClosed_(self, a):
            return True

    delegate = Delegate.alloc().init()
    app.setDelegate_(delegate)
    win.makeKeyAndOrderFront_(None)
    app.activateIgnoringOtherApps_(True)
    app.run()


if __name__ == "__main__":
    chat = sys.argv[1:] == ["chat"]
    if sys.platform == "darwin":
        mac_main(chat)
    else:
        gtk_main(chat)
