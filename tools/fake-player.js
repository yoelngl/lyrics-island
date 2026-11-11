// Fake MPRIS player for testing Lyrics Island without a real app.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const xml = `<node><interface name="org.mpris.MediaPlayer2.Player">
<method name="PlayPause"/><method name="Next"/><method name="Previous"/>
<property name="PlaybackStatus" type="s" access="read"/>
<property name="Metadata" type="a{sv}" access="read"/>
<property name="Position" type="x" access="read"/>
<signal name="Seeked"><arg type="x"/></signal>
</interface></node>`;

const start = GLib.get_monotonic_time() - 25_000_000; // pretend 25s in
const impl = {
    PlayPause() {
        print('PlayPause called');
    },
    Next() {},
    Previous() {},
    get PlaybackStatus() {
        return 'Playing';
    },
    get Metadata() {
        return {
            'mpris:trackid': new GLib.Variant('o', '/fake/1'),
            'xesam:title': new GLib.Variant('s', 'Blinding Lights'),
            'xesam:artist': new GLib.Variant('as', ['The Weeknd']),
            'xesam:album': new GLib.Variant('s', 'After Hours'),
            'mpris:length': new GLib.Variant('x', 200_000_000),
            'mpris:artUrl': new GLib.Variant('s', 'https://lrclib.net/favicon.ico'),
        };
    },
    get Position() {
        return GLib.get_monotonic_time() - start;
    },
};
const obj = Gio.DBusExportedObject.wrapJSObject(xml, impl);
obj.export(Gio.DBus.session, '/org/mpris/MediaPlayer2');
Gio.bus_own_name_on_connection(Gio.DBus.session, 'org.mpris.MediaPlayer2.youtube-music',
    Gio.BusNameOwnerFlags.NONE, () => print('fake player up'), null);
new GLib.MainLoop(null, false).run();
