export const effects = [
 ['Laser','Original rainbow foil and moving reflections.','laser'],
 ['Sunlight','Warm golden rays follow the angle of the card.','sunlight'],
 ['Moonlight','Silver-blue light brushes the shadowed surface.','moonlight'],
 ['Eclipse','Tilt to move the moon across a golden sun: crescent, totality and diamond ring.','eclipse'],
 ['Starlight','Pinpoint stars glint as you turn the card.','starlight'],
 ['Veiled Light','An angled window of gold reveals the artwork beneath.','veiled-light'],
 ['Prism','A beam splits into distinct spectral colors.','prism'],
 ['Aurora','Curtains of cyan, green and violet drift behind the guardian.','aurora'],
 ['Bioluminescence','Pixel seams glow from within.','bioluminescence'],
 ['Afterglow','Turning the card leaves fading phosphor echoes.','afterglow'],
 ['Caustics','Rippling pools of light flow across the surface.','caustics']
];
export const lightShader = `
uniform float uEffect;
uniform vec2 uMotion;
vec3 effectColor(float mode){
 if(mode<1.5)return vec3(1.,.65,.20);
 if(mode<2.5)return vec3(.40,.68,1.);
 if(mode<3.5)return vec3(1.,.48,.06);
 if(mode<4.5)return vec3(.65,.85,1.);
 if(mode<6.5)return vec3(1.,.72,.30);
 return vec3(.15,.85,.78);
}
vec3 illuminate(vec3 col,vec2 uv,vec2 su,vec4 sub,float line){
 float k=uFoil, t=uTime;
 vec2 v=uView.xy;
 float beam=uv.x*.85+uv.y*.42+v.x*.95+v.y*.45;
 vec3 gold=vec3(1.,.65,.18), ice=vec3(.38,.68,1.);
 if(uEffect<1.5){
  float ray=exp(-pow((beam-.57)*4.,2.));
  float shafts=.65+.35*pow(.5+.5*sin(uv.x*43.+uv.y*16.+v.x*8.),4.);
  col=col*(1.+k*ray*.32)+gold*k*ray*shafts*.36;
 }else if(uEffect<2.5){
  float ray=exp(-pow((uv.x-uv.y*.45+v.x*.7+v.y*.3-.22)*4.5,2.));
  float haze=exp(-pow((uv.x-uv.y*.45+v.x*.7+v.y*.3-.22)*2.2,2.));
  col*=mix(vec3(1.),vec3(.60,.83,1.12),min(k,1.));
  col+=ice*k*(ray*.72+haze*.13+line*sub.a*(.15+ray*.70));
 }else if(uEffect<3.5){
  // A fixed sun and a view-driven occluder produce actual crescent phases.
  vec2 p=(uv-vec2(.50,.67))*vec2(1.,1.5);
  vec2 moon=p-vec2(v.x*.65,v.y*.35);
  float r=length(p), mr=length(moon), radius=.225;
  float sun=1.-smoothstep(radius-.004,radius+.004,r);
  float cover=1.-smoothstep(radius-.005,radius+.005,mr);
  float crescent=sun*(1.-cover);
  float totality=exp(-length(vec2(v.x*.65,v.y*.35))*15.);
  float corona=exp(-abs(r-radius)*65.);
  float angle=atan(p.y,p.x);
  float rays=.7+.3*sin(angle*19.+t*.18)*sin(angle*31.-t*.12);
  float atmosphere=exp(-abs(r-radius)*15.)*(.35+totality*.65);
  col*=1.-min(k,1.)*cover*.88;
  // The disk is translucent over the print; light remains concentrated at its rim.
  col+=gold*k*(crescent*.75+corona*rays*(.32+totality*.8)+atmosphere*.16);
  float contact=exp(-pow((r-radius)*95.,2.))*exp(-pow((mr-radius)*95.,2.));
  col+=vec3(1.,.92,.65)*contact*k*1.5*(1.-totality);
 }else if(uEffect<4.5){
  vec2 q=(uv+v*.10)*vec2(15.,22.), id=floor(q), p=fract(q)-.5;
  float gate=step(.72,hash(id));
  float core=exp(-length(p)*32.);
  float cross=exp(-abs(p.x)*65.-abs(p.y)*7.)+exp(-abs(p.y)*65.-abs(p.x)*7.);
  float tw=.24+.76*pow(.5+.5*sin(t*1.4+hash(id)*30.+v.x*17.+v.y*13.),2.);
  col*=mix(vec3(1.),vec3(.8,.9,1.1),min(k,1.));
  col+=ice*k*gate*(core+cross*.85)*tw*2.8;
 }else if(uEffect<5.5){
  float reveal=exp(-pow((beam-.57)*8.,2.));
  col*=.07+reveal*1.35;col+=gold*k*reveal*.12;
 }else if(uEffect<6.5){
  float band=exp(-pow((beam-.60)*7.,2.));
  vec3 rgb=.5+.5*cos(6.283*(vec3(0.,.33,.67)+(beam-.42)*2.8));
  col+=rgb*k*band*.65;col=mix(col,col*(.65+rgb),band*k*.28);
 }else if(uEffect<7.5){
  float x=uv.x+v.x*.27;
  float bend=.43+.16*sin(uv.y*6.+t*.24)+.06*sin(uv.y*17.-t*.18);
  float curtain=exp(-pow((x-bend)*8.,2.))+.85*exp(-pow((x-bend-.22)*11.,2.));
  float folds=.55+.45*sin(x*110.+noise(uv*7.)*4.);
  vec3 aur=mix(vec3(.08,1.,.53),vec3(.52,.18,1.),.5+.5*sin(uv.y*7.+t*.2));
  col+=aur*k*curtain*(.34+folds*.58)*(1.-sub.a*.62);
  col+=aur*line*sub.a*k*.28;
 }else if(uEffect<8.5){
  float pulse=.70+.30*sin(t*1.5+uv.y*9.+v.x*8.);
  vec2 grid=floor(su*120.);
  float cells=smoothstep(.48,.70,noise(grid*.7));
  float sparks=step(.78,hash(grid));
  float detail=max(sub.r,max(sub.g,sub.b));
  col*=mix(vec3(1.),vec3(.65,.95,.9),min(k,1.));
  col+=vec3(.08,1.,.67)*k*sub.a*pulse*
       (line*.95+cells*(.14+detail*.26)+sparks*.16);
  // A second field makes the environment feel alive as well.
  vec2 q=(uv+v*.045)*vec2(42.,63.);
  vec2 f=fract(q)-.5;
  float motes=step(.73,hash(floor(q)))*exp(-length(f)*13.);
  col+=vec3(.12,.95,.78)*motes*k*pulse*.75*(1.-sub.a*.7);
 }else if(uEffect<9.5){
  vec2 trail=clamp(uMotion*1.2,vec2(-.035),vec2(.035));
  float moving=smoothstep(.0003,.008,length(uMotion));
  for(int i=1;i<=12;i++){
   vec2 p=su+trail*float(i);
   vec4 echo=texture2D(tSubject,clamp(p,0.,1.));
   float lum=max(echo.r,max(echo.g,echo.b));
   float fade=pow(1.-float(i)/13.,1.3);
   float scan=.65+.35*step(.45,fract(uv.y*420.));
   col+=mix(ice,gold,float(i)/12.)*max(0.,lum-.14)*echo.a*
        inside(p)*k*.52*fade*moving*scan;
  }
  col+=ice*line*sub.a*k*.30;
 }else{
  vec2 p=uv*9.+v*2.;
  p+=vec2(sin(p.y+t*.45),cos(p.x-t*.35))*.55;
  float net=abs(sin(p.x+p.y*.6+t*.28)*sin(p.y-p.x*.35-t*.23));
  float caustic=pow(1.-smoothstep(.0,.16,net),3.);
  col+=vec3(.35,.9,1.)*caustic*k*.35;
 }
 return col;
}
`;
