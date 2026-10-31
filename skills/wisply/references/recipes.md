# Wisply recipes

## Log in without leaking the password
```
wisply open example.com/login
wisply fill "Email=me@example.com" "Password=@secret" --submit --env SITE_PASS
wisply expect url:/dashboard
```
The password comes from an environment variable, never from the command line or the transcript.

## Read a logged-in dashboard with few tokens
1. `wisply open <dashboard>` and look at the snapshot.
2. Need numbers? Try `wisply table` or `wisply text -f revenue` before scrolling around.
3. The page loads its data from an API? `wisply fetch /api/... --pick ...` is often 10x smaller than the UI.

## Multi-step form in one call
```
wisply do "fill Name=Max Street=Main 1 City=Berlin; select e9 Germany; check e12; click Continue; expect Payment"
```
`do` stops at the first error and says which step failed.

## Upload and download
```
wisply upload e5 C:/Users/me/Desktop/logo.png
wisply click "Export CSV"
wisply downloads            (done 0.2 MB C:\Users\me\Downloads\export.csv)
```

## Work in the user's own tab
```
wisply tabs                 (find the tab, e.g. t2)
wisply tab t2
wisply snap
```
Only when the user asked for it. Afterwards `wisply open <url>` goes back to Wisply's own tab.

## Search results and long pages
- Direct URLs beat typing into search boxes: `wisply open "github.com/search?q=wisply&type=repositories"`.
- `wisply scroll --until "Next page"` scrolls in one call.
- `wisply snap -f "price"` instead of reading the whole page.

## Desktop app flow
```
wisply windows -f discord
wisply snap w4 -f message
wisply type d12 "hello"
wisply press Enter
```

## Game or canvas app
```
wisply read w6              (o1 "Singleplayer" @960,420 ...)
wisply click o1
wisply press w --game
```
