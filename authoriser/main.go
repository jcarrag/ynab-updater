package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path"
	"strings"
	"syscall"

	"github.com/BurntSushi/toml"
	"github.com/gregdel/pushover"
	"golang.org/x/oauth2/clientcredentials"
	"tailscale.com/tsnet"
)

// func StartFunnelServer(authKey string, passwordChan chan<- string) {
func StartFunnelServer(authKey string, password *string) {
	s := &tsnet.Server{Hostname: "ynab-updater", AuthKey: authKey}
	// defer s.Close()

	ln, err := s.ListenFunnel("tcp", ":443")
	if err != nil {
		log.Fatal(err)
		os.Exit(1)
	}
	// defer ln.Close()

	http.HandleFunc("GET /", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintln(w,
			`<html>
                         <meta name="viewport" content='width=device-width,initial-scale=1,maximum-scale=1' />
                         <form action='/' method='POST' style='height:50%;margin-top:25%;'>
                           <input id='password' name='password' type='password' style='width:100%;height:10%;margin-bottom:10px;font-size:30px;'/>
                           <button style='width:100%;height:10%;font-size:30px;'>Submit</button>
                         </form>`,
		)
	})

	http.HandleFunc("POST /", func(w http.ResponseWriter, r *http.Request) {
		defer ln.Close()
		// defer s.Close()

		r.ParseForm()
		// passwordChan <- r.FormValue("password")
		*password = r.FormValue("password")
		fmt.Fprintln(w, "<html><script type='text/javascript'>window.alert('Saved password');</script>")
		// FIXME: how to shutdown server without ending whole process
	})

	http.Serve(ln, nil)
}

func main() {
	type Config struct {
		OauthClientId     string `toml:"TS_OAUTH_CLIENT_ID"`
		OauthClientSecret string `toml:"TS_OAUTH_CLIENT_SECRET"`
		PushoverApiKey    string `toml:"PUSHOVER_API_KEY"`
		PushoverUserKey   string `toml:"PUSHOVER_USER_KEY"`
	}

	confPath := path.Join(os.Getenv("YNAB_CONFIG_PATH"),"/settings.toml")

	var conf Config
	_, err := toml.DecodeFile(confPath, &conf)
	if err != nil {
		log.Fatal("toml.DecodeFile error", err)
		os.Exit(1)
	}

	var oauthConfig = &clientcredentials.Config{
		ClientID:     conf.OauthClientId,
		ClientSecret: conf.OauthClientSecret,
		TokenURL:     "https://api.tailscale.com/api/v2/oauth/token",
	}

	client := oauthConfig.Client(context.Background())

	type CreateAuthKeyResponse struct {
		Id      string `json:"id"`
		Key     string `json:"key"`
		Created string `json:"created"`
		Expires string `json:"expires"`
	}

	// create authKey

	payload := strings.NewReader(`
                {
                  "capabilities": {
                    "devices": {
                      "create": {
                      "reusable": false,
                      "ephemeral": true,
                      "preauthorized": true,
                      "tags": ["tag:ynab-updater"]
                      }
                    }
                  },
                  "expirySeconds": 10,
                  "description": "ynab-updated_authoriser"
                }`)

	createAuthKeyResp, err := client.Post("https://api.tailscale.com/api/v2/tailnet/-/keys?all=true", "application/json", payload)
	if err != nil {
		log.Fatal("error creating authKey", err)
		os.Exit(1)
	}

	createAuthKeyBody, err := io.ReadAll(createAuthKeyResp.Body)
	if err != nil {
		log.Fatal("error reading response createAuthKeyBody", err)
		os.Exit(1)
	}

	fmt.Println("createAuthKeyBody:", string(createAuthKeyBody))

	createAuthKey := CreateAuthKeyResponse{}
	json.Unmarshal([]byte(createAuthKeyBody), &createAuthKey)

	// the password stored in memory

	// passwordChan := make(chan string, 1)
	password := ""

	// start funnel server

	go StartFunnelServer(createAuthKey.Key, &password) //passwordChan)

	// send pushover notification

	app := pushover.New(conf.PushoverApiKey)
	recipient := pushover.NewRecipient(conf.PushoverUserKey)
	message := &pushover.Message{
		Message:  "Log into ynab-updater",
		Title:    "Log in",
		URL:      "https://ynab-updater.tail7f031.ts.net/",
		URLTitle: "TS Funnel",
	}

	// Send the message to the recipient
	response, err := app.SendMessage(message, recipient)
	if err != nil {
		log.Fatal("error sending pushover message", err)
		os.Exit(1)
	}

	// Print the response if you want
	log.Println("pushover response", response)

	// start local server

	log.Printf("starting localhost server")
	const socketPath = "/tmp/ynab-updater_authoriser.sock"
	socket, err := net.Listen("unix", socketPath)
	if err != nil {
		log.Fatal("error opening unix socket", err)
		os.Exit(1)
	}

	// Cleanup the sockfile.
	c := make(chan os.Signal, 1)
	signal.Notify(c, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-c
		os.Remove(socketPath)
		os.Exit(1)
	}()

	localMux := http.NewServeMux()
	localMux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		// msg := "init"
		// select {
		// case password = <-passwordChan:
		//         msg = password
		// default: msg = "no password provided yet"
		// }
		// fmt.Fprintln(w, "<html>Password: ", password)
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		if password == "" {
			w.WriteHeader(404)
		} else {
			w.WriteHeader(200)
			bytes.NewBufferString(string(password)).WriteTo(w)
		}
	})

	localSrv := &http.Server{
		Handler: localMux,
	}

	localSrv.Serve(socket)

}
