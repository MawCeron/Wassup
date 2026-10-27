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
	want := &ipc.Settings{HideNotificationContent: true, MediaCacheMb: 200, GpuRendering: true}
	if _, err := d.setSettings(want); err != nil {
		t.Fatal(err)
	}
	c, err := loadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	if c != (config{HideNotificationContent: true, MediaCacheMB: 200, GPURendering: true}) {
		t.Fatalf("saved %+v", c)
	}
}
