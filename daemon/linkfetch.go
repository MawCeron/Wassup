package main

import (
	"bytes"
	"context"
	"errors"
	"image"
	"image/jpeg"
	"io"
	"mime"
	"net/http"
	"net/url"
	"strings"
	"time"

	_ "image/gif" // Formats a page's preview image may come in.
	_ "image/png"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"golang.org/x/net/html"
	"google.golang.org/protobuf/proto"
)

// Limits for fetching a link preview, so a slow or huge page cannot hold up
// sending or take much memory.
const (
	previewTimeout   = 5 * time.Second
	previewPageBytes = 512 << 10
	previewImgBytes  = 5 << 20
	previewImgPixels = 8_000_000 // About 12 MB while decoding.
	previewThumbSide = 200
)

var previewClient = &http.Client{Timeout: previewTimeout}

// firstURL finds the first web address in text by the same rule as the UI:
// a word starting with http://, https:// or www., without trailing
// punctuation. www. addresses get https://.
func firstURL(text string) (matched, fetch string) {
	for _, word := range strings.Fields(text) {
		lower := strings.ToLower(word)
		if !strings.HasPrefix(lower, "https://") && !strings.HasPrefix(lower, "http://") && !strings.HasPrefix(lower, "www.") {
			continue
		}
		word = strings.TrimRight(word, ".,;:!?)]\"'")
		if strings.HasPrefix(lower, "www.") {
			return word, "https://" + word
		}
		return word, word
	}
	return "", ""
}

// withLinkPreview turns a text with a web address into a message with the
// page's title, description and image, as the phone does. Without an
// address, or if the page cannot be read, the text goes as it is.
func withLinkPreview(ctx context.Context, text string) *waE2E.Message {
	plain := &waE2E.Message{Conversation: proto.String(text)}
	matched, address := firstURL(text)
	if address == "" {
		return plain
	}
	ctx, cancel := context.WithTimeout(ctx, previewTimeout)
	defer cancel()
	page, err := fetchPreview(ctx, address)
	if err != nil || page.title == "" {
		return plain
	}
	return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
		Text: proto.String(text), MatchedText: proto.String(matched),
		Title: proto.String(page.title), Description: proto.String(page.description),
		JPEGThumbnail: page.thumbnail,
		PreviewType:   waE2E.ExtendedTextMessage_NONE.Enum(),
	}}
}

type pagePreview struct {
	title, description string
	thumbnail          []byte
}

func fetchPreview(ctx context.Context, address string) (*pagePreview, error) {
	base, err := url.Parse(address)
	if err != nil || (base.Scheme != "https" && base.Scheme != "http") {
		return nil, errors.New("not a web address")
	}
	body, contentType, err := download(ctx, address, previewPageBytes)
	if err != nil {
		return nil, err
	}
	if media, _, _ := mime.ParseMediaType(contentType); media != "text/html" {
		return nil, errors.New("not a web page")
	}
	page, imageURL := parseHead(body)
	if imageURL != "" {
		if ref, err := base.Parse(imageURL); err == nil && (ref.Scheme == "https" || ref.Scheme == "http") {
			if data, _, err := download(ctx, ref.String(), previewImgBytes); err == nil {
				page.thumbnail = thumbnail(data)
			}
		}
	}
	return page, nil
}

// download fetches at most limit bytes of a page or image.
func download(ctx context.Context, address string, limit int64) ([]byte, string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, address, nil)
	if err != nil {
		return nil, "", err
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) Wassup")
	resp, err := previewClient.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, "", errors.New(resp.Status)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, limit))
	return data, resp.Header.Get("Content-Type"), err
}

// parseHead reads a page's title, description and preview image from its
// Open Graph tags, falling back to <title> and the description meta tag.
func parseHead(page []byte) (*pagePreview, string) {
	var p pagePreview
	var image, title, description string
	z := html.NewTokenizer(bytes.NewReader(page))
	inTitle := false
	for {
		switch z.Next() {
		case html.ErrorToken:
			return finish(&p, title, description), image
		case html.StartTagToken, html.SelfClosingTagToken:
			name, hasAttr := z.TagName()
			switch string(name) {
			case "body":
				return finish(&p, title, description), image
			case "title":
				inTitle = true
			case "meta":
				var key, content string
				for hasAttr {
					var k, v []byte
					k, v, hasAttr = z.TagAttr()
					switch string(k) {
					case "property", "name":
						key = strings.ToLower(string(v))
					case "content":
						content = strings.TrimSpace(string(v))
					}
				}
				switch key {
				case "og:title":
					p.title = content
				case "og:description":
					p.description = content
				case "description":
					description = content
				case "og:image", "og:image:url", "twitter:image":
					if image == "" {
						image = content
					}
				}
			}
		case html.TextToken:
			if inTitle && title == "" {
				title = strings.TrimSpace(string(z.Text()))
			}
		case html.EndTagToken:
			if name, _ := z.TagName(); string(name) == "title" {
				inTitle = false
			}
		}
	}
}

func finish(p *pagePreview, title, description string) *pagePreview {
	if p.title == "" {
		p.title = title
	}
	if p.description == "" {
		p.description = description
	}
	return p
}

// thumbnail shrinks an image to at most previewThumbSide pixels a side and
// encodes it as JPEG; nil if it cannot be read or is too big to decode.
func thumbnail(data []byte) []byte {
	cfg, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil || cfg.Width*cfg.Height > previewImgPixels || cfg.Width == 0 || cfg.Height == 0 {
		return nil
	}
	src, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil
	}
	b := src.Bounds()
	scale := min(1, float64(previewThumbSide)/float64(max(b.Dx(), b.Dy())))
	w, h := max(1, int(float64(b.Dx())*scale)), max(1, int(float64(b.Dy())*scale))
	dst := image.NewRGBA(image.Rect(0, 0, w, h))
	// Each output pixel averages the source pixels it covers.
	for y := range h {
		y0, y1 := b.Min.Y+y*b.Dy()/h, b.Min.Y+max((y+1)*b.Dy()/h, y*b.Dy()/h+1)
		for x := range w {
			x0, x1 := b.Min.X+x*b.Dx()/w, b.Min.X+max((x+1)*b.Dx()/w, x*b.Dx()/w+1)
			var r, g, bl, n uint64
			for sy := y0; sy < y1; sy++ {
				for sx := x0; sx < x1; sx++ {
					cr, cg, cb, _ := src.At(sx, sy).RGBA()
					r, g, bl, n = r+uint64(cr), g+uint64(cg), bl+uint64(cb), n+1
				}
			}
			i := dst.PixOffset(x, y)
			dst.Pix[i], dst.Pix[i+1], dst.Pix[i+2], dst.Pix[i+3] = uint8(r/n>>8), uint8(g/n>>8), uint8(bl/n>>8), 255
		}
	}
	var out bytes.Buffer
	if jpeg.Encode(&out, dst, &jpeg.Options{Quality: 70}) != nil {
		return nil
	}
	return out.Bytes()
}
