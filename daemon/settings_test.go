package main

import (
	"testing"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

func TestSettingsSaved(t *testing.T) {
	dir := t.TempDir()
	d := newDaemon(nil, nil)
	d.configDir = dir
	if got := d.settings().MediaCacheMb; got != defaultMediaCacheMB {
		t.Fatalf("default cache %d MB", got)
	}
	if _, err := d.setSettings(&ipc.Settings{Language: "../../etc"}); err == nil {
		t.Fatal("accepted a path as the language")
	}
	want := &ipc.Settings{HideNotificationContent: true, MediaCacheMb: 200, GpuRendering: true, Language: "pt_BR", TrayIcon: false}
	if _, err := d.setSettings(want); err != nil {
		t.Fatal(err)
	}
	c, err := loadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	if c != (config{HideNotificationContent: true, MediaCacheMB: 200, GPURendering: true, Language: "pt_BR", HideTrayIcon: true}) {
		t.Fatalf("saved %+v", c)
	}
}
