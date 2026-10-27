package main

import (
	"math"
	"path/filepath"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

// settings returns the preferences, with defaults filled in.
func (d *daemon) settings() *ipc.Settings {
	d.configMu.Lock()
	defer d.configMu.Unlock()
	mb := d.config.MediaCacheMB
	if mb <= 0 {
		mb = defaultMediaCacheMB
	}
	return &ipc.Settings{
		HideNotificationContent: d.config.HideNotificationContent,
		MediaCacheMb:            uint32(mb),
		GpuRendering:            d.config.GPURendering,
		Language:                d.config.Language,
	}
}

// setSettings saves the preferences and applies those the daemon uses.
func (d *daemon) setSettings(s *ipc.Settings) (*ipc.Settings, error) {
	c := config{
		HideNotificationContent: s.GetHideNotificationContent(),
		MediaCacheMB:            int(min(s.GetMediaCacheMb(), math.MaxInt32)),
		GPURendering:            s.GetGpuRendering(),
		Language:                s.GetLanguage(),
	}
	if err := c.validate(); err != nil {
		return nil, err
	}
	d.configMu.Lock()
	err := saveConfig(d.configDir, c)
	languageChanged := err == nil && c.Language != d.config.Language
	if err == nil {
		d.config = c
	}
	d.configMu.Unlock()
	if err != nil {
		return nil, err
	}
	if languageChanged {
		initLocale(c.Language)
	}
	if d.notify != nil {
		d.notify.hideContent.Store(c.HideNotificationContent)
	}
	for _, a := range d.accounts {
		a.mediaCacheMB.Store(int32(c.MediaCacheMB))
		// A smaller cache takes effect now, not at the next download.
		if a.mediaDir != "" {
			go evictMedia(filepath.Dir(a.mediaDir), a.cacheLimit())
		}
	}
	return d.settings(), nil
}
