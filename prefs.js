import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class LyricsIslandPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage();

        const timing = new Adw.PreferencesGroup({title: 'Timing'});
        const appearance = new Adw.PreferencesGroup({title: 'Appearance'});

        const spin = (group, key, title, subtitle, lower, upper, step) => {
            const row = Adw.SpinRow.new_with_range(lower, upper, step);
            row.title = title;
            row.subtitle = subtitle;
            settings.bind(key, row, 'value', Gio.SettingsBindFlags.DEFAULT);
            group.add(row);
        };

        spin(timing, 'lyric-offset', 'Lyric offset (ms)', 'Positive values show lines earlier', -5000, 5000, 50);
        spin(appearance, 'max-width', 'Max compact width (px)', 'Widest the pill grows for long lines', 200, 900, 10);
        spin(appearance, 'font-size', 'Lyric font size (pt)', 'Text size in the compact pill', 8, 18, 1);

        const themes = ['system', 'dark', 'light'];
        const theme = new Adw.ComboRow({
            title: 'Theme',
            subtitle: 'System follows the desktop Light/Dark style',
            model: Gtk.StringList.new(['Follow system', 'Dark', 'Light']),
            selected: Math.max(0, themes.indexOf(settings.get_string('theme'))),
        });
        theme.connect('notify::selected', () => settings.set_string('theme', themes[theme.selected]));
        appearance.add(theme);

        page.add(timing);
        page.add(appearance);
        window.add(page);
        window._settings = settings;
    }
}
