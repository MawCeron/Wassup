package main

import (
	"bytes"
	"context"
	"image"
	"image/color"
	"image/jpeg"
	"image/png"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestFirstURL(t *testing.T) {
	for text, want := range map[string][2]string{
		"see https://gnome.org/apps, ok": {"https://gnome.org/apps", "https://gnome.org/apps"},
		"WWW.example.com.":               {"WWW.example.com", "https://WWW.example.com"},
		"nohttps://here":                 {"", ""},
		"no links here":                  {"", ""},
	} {
		if m, f := firstURL(text); m != want[0] || f != want[1] {
			t.Errorf("firstURL(%q) = %q, %q; want %q", text, m, f, want)
		}
	}
}

func TestLinkPreview(t *testing.T) {
	big := image.NewRGBA(image.Rect(0, 0, 800, 400))
	for i := range big.Pix {
		big.Pix[i] = 200
	}
	big.Set(0, 0, color.Black)
	var pngData bytes.Buffer
	png.Encode(&pngData, big)
	mux := http.NewServeMux()
	mux.HandleFunc("/og", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Write([]byte(`<html><head><title>Fallback</title>
			<meta property="og:title" content="GNOME News">
			<meta property="og:description" content="The latest">
			<meta property="og:image" content="/img.png"></head><body>x</body></html>`))
	})
	mux.HandleFunc("/plain", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html")
		w.Write([]byte(`<title> Just a title </title><meta name="description" content="Desc">`))
	})
	mux.HandleFunc("/img.png", func(w http.ResponseWriter, r *http.Request) { w.Write(pngData.Bytes()) })
	mux.HandleFunc("/file", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/pdf")
		w.Write([]byte("%PDF"))
	})
	srv := httptest.NewServer(mux)
	defer srv.Close()
	ctx := context.Background()

	m := withLinkPreview(ctx, "mira "+srv.URL+"/og!")
	e := m.GetExtendedTextMessage()
	if e.GetTitle() != "GNOME News" || e.GetDescription() != "The latest" || e.GetMatchedText() != srv.URL+"/og" || e.GetText() != "mira "+srv.URL+"/og!" {
		t.Fatalf("og preview: %v", e)
	}
	thumb, err := jpeg.DecodeConfig(bytes.NewReader(e.GetJPEGThumbnail()))
	if err != nil || thumb.Width != 200 || thumb.Height != 100 {
		t.Fatalf("thumbnail %dx%d, %v; want 200x100", thumb.Width, thumb.Height, err)
	}
	e = withLinkPreview(ctx, srv.URL+"/plain").GetExtendedTextMessage()
	if e.GetTitle() != "Just a title" || e.GetDescription() != "Desc" || e.GetJPEGThumbnail() != nil {
		t.Fatalf("fallback preview: %v", e)
	}
	for _, text := range []string{srv.URL + "/file", srv.URL + "/missing", "hola"} {
		if m := withLinkPreview(ctx, text); m.GetConversation() != text {
			t.Fatalf("%q should go as plain text: %v", text, m)
		}
	}
}
