# Lighting recipes

## Log in without leaking the password
```
lighting open example.com/login
lighting fill "Email=me@example.com" "Password=@secret" --submit --env SITE_PASS
lighting expect url:/dashboard
```
The password comes from an environment variable, never from the command line or the transcript.

## Read a logged-in dashboard with few tokens
1. `lighting open <dashboard>` and look at the snapshot.
2. Need numbers? Try `lighting table` or `lighting text -f revenue` before scrolling around.
3. The page loads its data from an API? `lighting fetch /api/... --pick ...` is often 10x smaller than the UI.

## Multi-step form in one call
```
lighting do "fill Name=Max Street=Main 1 City=Berlin; select e9 Germany; check e12; click Continue; expect Payment"
```
`do` stops at the first error and says which step failed.

## Upload and download
```
lighting upload e5 C:/Users/me/Desktop/logo.png
lighting click "Export CSV"
lighting downloads            (done 0.2 MB C:\Users\me\Downloads\export.csv)
```

## Work in the user's own tab
```
lighting tabs                 (find the tab, e.g. t2)
lighting tab t2
lighting snap
```
Only when the user asked for it. Afterwards `lighting open <url>` goes back to Lighting's own tab.

## Search results and long pages
- Direct URLs beat typing into search boxes: `lighting open "github.com/search?q=lighting&type=repositories"`.
- `lighting scroll --until "Next page"` scrolls in one call.
- `lighting snap -f "price"` instead of reading the whole page.

## Desktop app flow
```
lighting windows -f discord
lighting snap w4 -f message
lighting type d12 "hello"
lighting press Enter
```

## Game or canvas app
```
lighting read w6              (o1 "Singleplayer" @960,420 ...)
lighting click o1
lighting press w --game
```
