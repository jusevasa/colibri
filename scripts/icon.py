"""Genera el icono de Colibrí: assets/colibri.svg y assets/colibri-1024.png.

Uso: python3 scripts/icon.py assets   (luego scripts/bundle.sh lo empaqueta)
El .icns se arma con sips + iconutil a partir del PNG.
"""
import zlib, struct, sys
GRID = [
"................",
"................",
"..ww............",
"..www...........",
"...www....gg....",
"...wwww..gggg...",
"....wwwwggegbbbb",
"....wwwggggr....",
"....wwggggrr....",
"...ggggggll.....",
"..ggggggll......",
".ttgggggl.......",
"ttt.ggg.........",
"tt..............",
"................",
"................",
]
COL = {'w':'#1f9e8f','g':'#34d6a8','e':'#0b0e14','b':'#e8ecf1','r':'#ff5d8f','l':'#c9f7e6','t':'#167a70'}
BG_TOP, BG_BOT = '#161c27', '#0a0d13'
N=16
def rgb(h): return tuple(int(h[i:i+2],16) for i in (1,3,5))
# Lienzo 1024: baldosa de 824 con radio 185 (plantilla de iconos de macOS), rejilla de 624.
TILE, M, R = 824, 100, 185
CELL = 39; G0 = (1024 - CELL*N)//2
def svg():
    out=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" shape-rendering="crispEdges">',
         '<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="%s"/><stop offset="1" stop-color="%s"/></linearGradient></defs>'%(BG_TOP,BG_BOT),
         '<rect x="%d" y="%d" width="%d" height="%d" rx="%d" fill="url(#bg)" shape-rendering="geometricPrecision"/>'%(M,M,TILE,TILE,R)]
    for y,row in enumerate(GRID):
        x=0
        while x<N:
            c=row[x]
            if c=='.': x+=1; continue
            x2=x
            while x2<N and row[x2]==c: x2+=1
            out.append('<rect x="%d" y="%d" width="%d" height="%d" fill="%s"/>'%(G0+x*CELL,G0+y*CELL,(x2-x)*CELL,CELL,COL[c]))
            x=x2
    out.append('</svg>')
    return '\n'.join(out)
def png(path, size=1024, ss=2):
    W=size*ss; k=W/1024
    top,bot=rgb(BG_TOP),rgb(BG_BOT)
    rows=[]
    m,t,r=M*k,TILE*k,R*k
    for y in range(W):
        row=bytearray()
        fy=(y-m)/t
        bgc=tuple(int(top[i]+(bot[i]-top[i])*min(max(fy,0),1)) for i in range(3))
        gy=int((y/k-G0)//CELL)
        for x in range(W):
            # dentro del rectángulo redondeado
            inside = m<=x<m+t and m<=y<m+t
            if inside:
                cx=min(max(x,m+r),m+t-r); cy=min(max(y,m+r),m+t-r)
                inside=(x-cx)**2+(y-cy)**2<=r*r
            if not inside: row+=b'\0\0\0\0'; continue
            gx=int((x/k-G0)//CELL)
            c=GRID[gy][gx] if 0<=gx<N and 0<=gy<N else '.'
            col=rgb(COL[c]) if c!='.' else bgc
            row+=bytes(col)+b'\xff'
        rows.append(bytes(row))
    # reducir ss×ss promediando (suaviza el borde redondeado)
    out=bytearray()
    for y in range(size):
        out.append(0)
        for x in range(size):
            acc=[0,0,0,0]
            for dy in range(ss):
                rr=rows[y*ss+dy]
                for dx in range(ss):
                    o=(x*ss+dx)*4; a=rr[o+3]
                    acc[0]+=rr[o]*a; acc[1]+=rr[o+1]*a; acc[2]+=rr[o+2]*a; acc[3]+=a
            a=acc[3]
            out+=bytes((acc[0]//a if a else 0, acc[1]//a if a else 0, acc[2]//a if a else 0, a//(ss*ss)))
    def chunk(t,d): return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d)&0xffffffff)
    open(path,'wb').write(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',size,size,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(bytes(out),9))+chunk(b'IEND',b''))
if __name__=='__main__':
    d=sys.argv[1]
    open(d+'/colibri.svg','w').write(svg())
    png(d+'/colibri-1024.png')
