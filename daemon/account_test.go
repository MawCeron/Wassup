package main

import (
	"context"
	"testing"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

func TestOpenAccountsReusesStore(t *testing.T) {
	dir := t.TempDir()
	first, err := openAccounts(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	closeAccounts(first)
	if len(first) != 1 {
		t.Fatalf("got %d accounts in an empty data dir, want 1", len(first))
	}

	again, err := openAccounts(context.Background(), dir)
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(again)
	if len(again) != 1 || again[0].id != first[0].id {
		t.Fatalf("reopen created a new account instead of reusing %s", first[0].id)
	}
	if got := again[0].info().State; got != ipc.ConnectionState_CONNECTION_STATE_LOGGED_OUT {
		t.Fatalf("unlinked account state = %v", got)
	}
}
