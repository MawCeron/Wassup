package main

import (
	"context"
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
		TrayIcon:                !d.config.HideTrayIcon,
		LinkPreviews:            !d.config.NoLinkPreviews,
		ChatBackground:          d.config.ChatBackground,
	}
}

// refreshTray shows the current unread count on the tray icon.
func (d *daemon) refreshTray() {
	total := 0
	for _, a := range d.accounts {
		total += a.unreadCount(context.Background())
	}
	d.tray.setUnread(total)
}

// setSettings saves the preferences and applies those the daemon uses.
func (d *daemon) setSettings(s *ipc.Settings) (*ipc.Settings, error) {
	c := config{
		HideNotificationContent: s.GetHideNotificationContent(),
		MediaCacheMB:            int(min(s.GetMediaCacheMb(), math.MaxInt32)),
		GPURendering:            s.GetGpuRendering(),
		Language:                s.GetLanguage(),
		HideTrayIcon:            !s.GetTrayIcon(),
		NoLinkPreviews:          !s.GetLinkPreviews(),
		ChatBackground:          s.GetChatBackground(),
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
	d.tray.setEnabled(!c.HideTrayIcon)
	d.refreshTray()
	if d.notify != nil {
		d.notify.hideContent.Store(c.HideNotificationContent)
	}
	for _, a := range d.accounts {
		a.mediaCacheMB.Store(int32(c.MediaCacheMB))
		a.linkPreviews.Store(!c.NoLinkPreviews)
		// A smaller cache takes effect now, not at the next download.
		if a.mediaDir != "" {
			go evictMedia(filepath.Dir(a.mediaDir), a.cacheLimit())
		}
	}
	return d.settings(), nil
}
